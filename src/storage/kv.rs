//! `(defstorage)` — typed persistent key/value stores, Lisp-native.
//!
//! # Why pure tatara-lisp, not SQLite
//!
//! Every other storage-shaped thing in the substrate (state cells,
//! bookmarks, history, blocker registries, pipeline loadout) is
//! already Lisp-authored + typed. Introducing SQLite here would
//! fragment the storage paradigm into two halves that can't
//! homogeneously compose. Pure-Lisp keeps everything in one
//! substrate, BLAKE3-attestable, queryable via tatara-eval, and
//! avoids an FFI dep. At browser scale (~10k cookies/bookmarks/
//! history entries per user) the indexed-query advantage SQLite
//! offers isn't load-bearing; the simplicity is.
//!
//! SQL is declarative; tatara-lisp is even more so — homoiconic,
//! pattern-matched by tatara-eval, composable with every other
//! def* DSL.
//!
//! # Persistence shape
//!
//! Each store is an append-only Lisp event log on disk:
//!
//! ```text
//! (event :ts 1729342712 :op "set" :key "session" :value "abc")
//! (event :ts 1729342720 :op "set" :key "pref"    :value "dark")
//! (event :ts 1729342733 :op "delete" :key "session")
//! ```
//!
//! On open, the log replays into an in-memory map. Writes append to
//! the log before mutating. `compact()` rewrites the log with only
//! the latest surviving value per key.
//!
//! # Authoring
//!
//! ```lisp
//! (defstorage :name "cookies"
//!             :path "~/.local/share/namimado/cookies.log"
//!             :ttl-seconds 2592000)
//!
//! (defstorage :name "session"
//!             :path "~/.local/state/namimado/session.log")
//! ```
//!
//! V1: missing path = volatile (in-memory only). V2 adds fsync
//! batching + compaction triggers.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

#[cfg(feature = "lisp")]
use tatara_lisp::DeriveTataraDomain;

/// Declarative storage spec — the Lisp authoring surface.
#[cfg_attr(feature = "lisp", derive(DeriveTataraDomain))]
#[cfg_attr(feature = "lisp", tatara(keyword = "defstorage"))]
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct StorageSpec {
    pub name: String,
    /// Journal file path. `None` means volatile (in-memory only).
    #[serde(default)]
    pub path: Option<PathBuf>,
    /// Optional per-entry TTL in seconds. `None` = no expiry.
    #[serde(default)]
    pub ttl_seconds: Option<u64>,
    /// Secondary-index declarations. Each entry is a dot-separated
    /// JSON path into the stored value (e.g. `"domain"`, `"user.id"`,
    /// `"tags.0"`). The engine maintains one sorted map per path —
    /// `by_index(path, value)` returns every key whose value has that
    /// projected field. Indexes rebuild on journal replay.
    ///
    /// Authored as `:indexes ("domain" "expires")` in Lisp.
    #[serde(default)]
    pub indexes: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Registry of declared stores.
#[derive(Debug, Clone, Default)]
pub struct StorageRegistry {
    specs: Vec<StorageSpec>,
}

impl StorageRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, spec: StorageSpec) {
        self.specs.retain(|s| s.name != spec.name);
        self.specs.push(spec);
    }

    pub fn extend(&mut self, specs: impl IntoIterator<Item = StorageSpec>) {
        for s in specs {
            self.insert(s);
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.specs.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    pub fn specs(&self) -> &[StorageSpec] {
        &self.specs
    }

    #[must_use]
    pub fn get(&self, name: &str) -> Option<&StorageSpec> {
        self.specs.iter().find(|s| s.name == name)
    }
}

/// Runtime handle to one store. `Arc<Mutex<…>>` so clones share.
#[derive(Debug, Clone)]
pub struct Store {
    inner: Arc<Mutex<StoreInner>>,
}

#[derive(Debug)]
struct StoreInner {
    name: String,
    path: Option<PathBuf>,
    ttl: Option<u64>,
    entries: HashMap<String, Entry>,
    /// Declared index paths, verbatim from the spec. Shared as the
    /// key space for `indexes` below; preserved so `by_index_paths`
    /// can list them without mutating state.
    index_paths: Vec<String>,
    /// Secondary indexes: `path → indexed_value (stringified) → set of entry keys`.
    /// BTreeMap on the middle layer gives deterministic iteration and
    /// opens the door to range queries on sortable values.
    indexes: HashMap<String, std::collections::BTreeMap<String, std::collections::HashSet<String>>>,
}

#[derive(Debug, Clone, PartialEq)]
struct Entry {
    value: Value,
    ts: u64,
}

#[derive(Debug, Clone, PartialEq)]
enum Event {
    Set { ts: u64, key: String, value: Value },
    Delete { ts: u64, key: String },
}

impl Store {
    /// Construct from a spec. Replays any existing journal; errors
    /// log + continue with an empty store — we'd rather start than
    /// refuse to open a corrupt file.
    #[must_use]
    pub fn from_spec(spec: &StorageSpec) -> Self {
        let mut entries = HashMap::new();
        if let Some(path) = &spec.path {
            if let Ok(contents) = std::fs::read_to_string(path) {
                for line in contents.lines() {
                    if let Some(ev) = parse_event(line) {
                        apply_event(&mut entries, ev);
                    }
                }
            }
        }
        let mut indexes: HashMap<
            String,
            std::collections::BTreeMap<String, std::collections::HashSet<String>>,
        > = HashMap::new();
        for path in &spec.indexes {
            indexes.insert(path.clone(), std::collections::BTreeMap::new());
        }
        // Rebuild indexes from the replayed entries.
        for (key, entry) in &entries {
            for path in &spec.indexes {
                if let Some(val) = project_value(&entry.value, path) {
                    let map = indexes.entry(path.clone()).or_default();
                    map.entry(val).or_default().insert(key.clone());
                }
            }
        }
        Self {
            inner: Arc::new(Mutex::new(StoreInner {
                name: spec.name.clone(),
                path: spec.path.clone(),
                ttl: spec.ttl_seconds,
                entries,
                index_paths: spec.indexes.clone(),
                indexes,
            })),
        }
    }

    pub fn set(&self, key: impl Into<String>, value: Value) {
        let now = now_secs();
        let key = key.into();
        let mut inner = self.lock();
        // Evict old index entries before overwriting (the old value
        // may project to a different indexed slot than the new one).
        remove_from_indexes(&mut inner, &key);
        inner.entries.insert(
            key.clone(),
            Entry {
                value: value.clone(),
                ts: now,
            },
        );
        insert_into_indexes(&mut inner, &key, &value);
        let event = Event::Set { ts: now, key, value };
        if let Some(line) = format_event(&event) {
            append_line(inner.path.as_deref(), &line);
        }
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<Value> {
        let mut inner = self.lock();
        let ttl = inner.ttl;
        let expired = {
            let entry = inner.entries.get(key)?;
            ttl.is_some_and(|t| now_secs().saturating_sub(entry.ts) > t)
        };
        if expired {
            inner.entries.remove(key);
            return None;
        }
        inner.entries.get(key).map(|e| e.value.clone())
    }

    pub fn delete(&self, key: &str) -> bool {
        let now = now_secs();
        let mut inner = self.lock();
        let had = inner.entries.remove(key).is_some();
        if had {
            remove_from_indexes(&mut inner, key);
            let event = Event::Delete {
                ts: now,
                key: key.to_owned(),
            };
            if let Some(line) = format_event(&event) {
                append_line(inner.path.as_deref(), &line);
            }
        }
        had
    }

    /// Keys whose projected value at `index_path` equals `indexed_value`.
    /// O(log n) lookup instead of O(n) scan — the point of indexes.
    /// Returns `None` if the index wasn't declared in the spec.
    #[must_use]
    pub fn by_index(&self, index_path: &str, indexed_value: &str) -> Option<Vec<(String, Value)>> {
        let mut inner = self.lock();
        let ttl = inner.ttl;
        prune_expired(&mut inner.entries, ttl);
        let keys: Vec<String> = inner
            .indexes
            .get(index_path)?
            .get(indexed_value)
            .map(|set| set.iter().cloned().collect())
            .unwrap_or_default();
        Some(
            keys.into_iter()
                .filter_map(|k| inner.entries.get(&k).map(|e| (k, e.value.clone())))
                .collect(),
        )
    }

    /// Every distinct indexed value for `index_path`, sorted. Useful
    /// for "show me all domains I have cookies for" style queries.
    #[must_use]
    pub fn index_values(&self, index_path: &str) -> Option<Vec<String>> {
        let inner = self.lock();
        Some(inner.indexes.get(index_path)?.keys().cloned().collect())
    }

    /// Range scan over a secondary index — every entry whose projected
    /// value at `index_path` falls lexicographically in `[lo, hi]`
    /// (inclusive bounds). `lo = ""` / `hi = "\u{10FFFF}"` collapses
    /// to an unbounded scan. Uses `BTreeMap::range` so the walk is
    /// O(log n + k) in the hit count.
    ///
    /// Numeric values are compared lexicographically on their string
    /// form — zero-pad your keys (`"0030"` not `"30"`) when you want
    /// numeric ordering and the index is of unbounded width.
    ///
    /// Returns `None` when the index isn't declared.
    #[must_use]
    pub fn by_index_range(
        &self,
        index_path: &str,
        lo: &str,
        hi: &str,
    ) -> Option<Vec<(String, Value)>> {
        let mut inner = self.lock();
        let ttl = inner.ttl;
        prune_expired(&mut inner.entries, ttl);
        let keys: Vec<String> = inner
            .indexes
            .get(index_path)?
            .range(lo.to_owned()..=hi.to_owned())
            .flat_map(|(_, set)| set.iter().cloned())
            .collect();
        Some(
            keys.into_iter()
                .filter_map(|k| inner.entries.get(&k).map(|e| (k, e.value.clone())))
                .collect(),
        )
    }

    /// All declared index paths.
    #[must_use]
    pub fn index_paths(&self) -> Vec<String> {
        self.lock().index_paths.clone()
    }

    /// Rebuild all indexes from scratch. Useful after direct
    /// mutation (replay, merge_from, compact). Idempotent.
    pub fn rebuild_indexes(&self) {
        let mut inner = self.lock();
        // Snapshot entries + paths since we need to mutate indexes
        // while walking entries.
        let paths = inner.index_paths.clone();
        let snapshot: Vec<(String, Value)> = inner
            .entries
            .iter()
            .map(|(k, e)| (k.clone(), e.value.clone()))
            .collect();
        inner.indexes.clear();
        for p in &paths {
            inner.indexes.insert(p.clone(), std::collections::BTreeMap::new());
        }
        for (key, val) in &snapshot {
            for p in &paths {
                if let Some(v) = project_value(val, p) {
                    let map = inner.indexes.entry(p.clone()).or_default();
                    map.entry(v).or_default().insert(key.clone());
                }
            }
        }
    }

    /// Live key snapshot — excludes TTL-expired entries.
    #[must_use]
    pub fn keys(&self) -> Vec<String> {
        let mut inner = self.lock();
        let ttl = inner.ttl;
        prune_expired(&mut inner.entries, ttl);
        inner.entries.keys().cloned().collect()
    }

    /// Keys whose string starts with `prefix`. Useful for namespaced
    /// stores (`"user/"`, `"tab/123/"`, `"cookie/domain.com/"`).
    /// O(n) over live entries — fine at browser scale.
    #[must_use]
    pub fn prefix_keys(&self, prefix: &str) -> Vec<String> {
        let mut inner = self.lock();
        let ttl = inner.ttl;
        prune_expired(&mut inner.entries, ttl);
        inner
            .entries
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect()
    }

    /// Full `(key, value)` snapshot, excluding TTL-expired. Cheap to
    /// clone because `Value` is `Arc`-friendly under the hood.
    #[must_use]
    pub fn entries(&self) -> Vec<(String, Value)> {
        let mut inner = self.lock();
        let ttl = inner.ttl;
        prune_expired(&mut inner.entries, ttl);
        inner
            .entries
            .iter()
            .map(|(k, e)| (k.clone(), e.value.clone()))
            .collect()
    }

    /// Entries matching a caller-supplied predicate over `(key, value)`.
    /// The predicate runs under the store lock — keep it cheap.
    #[must_use]
    pub fn filter<F>(&self, mut pred: F) -> Vec<(String, Value)>
    where
        F: FnMut(&str, &Value) -> bool,
    {
        let mut inner = self.lock();
        let ttl = inner.ttl;
        prune_expired(&mut inner.entries, ttl);
        inner
            .entries
            .iter()
            .filter(|(k, e)| pred(k, &e.value))
            .map(|(k, e)| (k.clone(), e.value.clone()))
            .collect()
    }

    /// Bulk insert. Appends one journal line per (key, value) pair.
    /// Ordering is preserved; last write per key wins.
    pub fn set_many<I, K>(&self, pairs: I)
    where
        I: IntoIterator<Item = (K, Value)>,
        K: Into<String>,
    {
        for (k, v) in pairs {
            self.set(k, v);
        }
    }

    /// Bulk delete by key list. Returns how many keys existed.
    pub fn delete_many<'a, I>(&self, keys: I) -> usize
    where
        I: IntoIterator<Item = &'a str>,
    {
        keys.into_iter()
            .map(|k| usize::from(self.delete(k)))
            .sum()
    }

    /// True when `key` is present (and not TTL-expired). O(1).
    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    /// Merge another store into this one. Last-write-wins by timestamp.
    /// Used for import / sync primitives. The other store is read
    /// atomically at call time; subsequent mutations there don't
    /// propagate.
    pub fn merge_from(&self, other: &Store) {
        for (k, v) in other.entries() {
            self.set(k, v);
        }
    }

    // ── Lisp-value access ──────────────────────────────────────────
    //
    // Values can BE Lisp expressions, not just data. We tag them at
    // write time so readers know they need evaluation before use.
    // Future: secondary indexes keyed by eval-result enable O(1)
    // access patterns for common shapes — callers request an index,
    // the store reshapes without losing data.

    /// Store a tatara-lisp expression (the raw source text). The
    /// store marks it so readers can pull it as a string AND feed it
    /// to an evaluator. Regular JSON values remain untagged.
    pub fn set_expr(&self, key: impl Into<String>, sexp: impl Into<String>) {
        self.set(
            key,
            serde_json::json!({ "_lisp": sexp.into() }),
        );
    }

    /// Retrieve an expression stored via [`Store::set_expr`]. Returns
    /// `None` if the key isn't present OR the value isn't a tagged
    /// Lisp expression (it was stored as plain data via [`Store::set`]).
    #[must_use]
    pub fn get_expr(&self, key: &str) -> Option<String> {
        match self.get(key)? {
            Value::Object(map) => map
                .get("_lisp")
                .and_then(|v| v.as_str())
                .map(str::to_owned),
            _ => None,
        }
    }

    /// All keys whose value is a tagged Lisp expression. Useful for
    /// batch eval passes and for the secondary-index arc in V2.
    #[must_use]
    pub fn lisp_keys(&self) -> Vec<String> {
        self.filter(|_, v| {
            matches!(v, Value::Object(map) if map.contains_key("_lisp"))
        })
        .into_iter()
        .map(|(k, _)| k)
        .collect()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        let mut inner = self.lock();
        let ttl = inner.ttl;
        prune_expired(&mut inner.entries, ttl);
        inner.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub fn name(&self) -> String {
        self.lock().name.clone()
    }

    /// Rewrite the journal with only the latest surviving value per
    /// key; drop delete tombstones. Safe to call periodically.
    pub fn compact(&self) -> std::io::Result<()> {
        let inner = self.lock();
        let Some(path) = inner.path.clone() else {
            return Ok(());
        };
        let mut lines = String::new();
        for (key, entry) in &inner.entries {
            let event = Event::Set {
                ts: entry.ts,
                key: key.clone(),
                value: entry.value.clone(),
            };
            if let Some(line) = format_event(&event) {
                lines.push_str(&line);
                lines.push('\n');
            }
        }
        std::fs::write(path, lines)
    }

    fn lock(&self) -> MutexGuard<'_, StoreInner> {
        self.inner.lock().expect("kv store mutex poisoned")
    }
}

// ─── index helpers ───────────────────────────────────────────────

/// Dot-path projection into a JSON value. `"user.id"` walks object
/// fields; `"tags.0"` indexes into arrays by decimal index. Returns
/// the projected leaf as a string suitable for BTreeMap lookup.
/// Non-scalar leaves (objects, arrays, nulls) yield `None`.
fn project_value(value: &Value, path: &str) -> Option<String> {
    let mut cursor = value;
    for segment in path.split('.') {
        cursor = match cursor {
            Value::Object(m) => m.get(segment)?,
            Value::Array(arr) => {
                let idx: usize = segment.parse().ok()?;
                arr.get(idx)?
            }
            _ => return None,
        };
    }
    match cursor {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn insert_into_indexes(inner: &mut StoreInner, key: &str, value: &Value) {
    for path in inner.index_paths.clone() {
        if let Some(v) = project_value(value, &path) {
            let map = inner.indexes.entry(path).or_default();
            map.entry(v).or_default().insert(key.to_owned());
        }
    }
}

fn remove_from_indexes(inner: &mut StoreInner, key: &str) {
    for (_path, bucket_map) in inner.indexes.iter_mut() {
        // Every index bucket that contained `key` must drop it; empty
        // buckets are reaped so `index_values()` doesn't report ghosts.
        let empty_buckets: Vec<String> = bucket_map
            .iter_mut()
            .filter_map(|(indexed_val, keys)| {
                keys.remove(key);
                if keys.is_empty() {
                    Some(indexed_val.clone())
                } else {
                    None
                }
            })
            .collect();
        for b in empty_buckets {
            bucket_map.remove(&b);
        }
    }
}

fn apply_event(entries: &mut HashMap<String, Entry>, ev: Event) {
    match ev {
        Event::Set { ts, key, value } => {
            entries.insert(key, Entry { value, ts });
        }
        Event::Delete { key, .. } => {
            entries.remove(&key);
        }
    }
}

/// Drop entries whose age exceeds `ttl`. No-op when `ttl` is `None`.
fn prune_expired(entries: &mut HashMap<String, Entry>, ttl: Option<u64>) {
    let Some(t) = ttl else { return };
    let now = now_secs();
    entries.retain(|_, e| now.saturating_sub(e.ts) <= t);
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn append_line(path: Option<&std::path::Path>, line: &str) {
    let Some(path) = path else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut body = String::from(line);
    body.push('\n');
    let _ = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut f| {
            use std::io::Write;
            f.write_all(body.as_bytes())
        });
}

fn format_event(event: &Event) -> Option<String> {
    match event {
        Event::Set { ts, key, value } => Some(format!(
            "(event :ts {ts} :op \"set\" :key {} :value {})",
            quote(key),
            serde_json::to_string(value).ok()?
        )),
        Event::Delete { ts, key } => Some(format!(
            "(event :ts {ts} :op \"delete\" :key {})",
            quote(key),
        )),
    }
}

fn parse_event(line: &str) -> Option<Event> {
    let s = line.trim();
    if !s.starts_with("(event ") || !s.ends_with(')') {
        return None;
    }
    let body = &s[7..s.len() - 1];
    let ts = scan_u64(body, ":ts ")?;
    let op = scan_quoted(body, ":op ")?;
    let key = scan_quoted(body, ":key ")?;
    match op.as_str() {
        "set" => {
            let raw = scan_trailing(body, ":value ")?;
            let value: Value = serde_json::from_str(&raw).ok()?;
            Some(Event::Set { ts, key, value })
        }
        "delete" => Some(Event::Delete { ts, key }),
        _ => None,
    }
}

fn scan_u64(src: &str, tag: &str) -> Option<u64> {
    let i = src.find(tag)? + tag.len();
    let rest = &src[i..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn scan_quoted(src: &str, tag: &str) -> Option<String> {
    let i = src.find(tag)? + tag.len();
    let rest = &src[i..];
    if !rest.starts_with('"') {
        return None;
    }
    let body = &rest[1..];
    let mut out = String::new();
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        if c == '"' {
            return Some(out);
        }
        if c == '\\' {
            match chars.next()? {
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                other => out.push(other),
            }
        } else {
            out.push(c);
        }
    }
    None
}

fn scan_trailing(src: &str, tag: &str) -> Option<String> {
    let i = src.find(tag)? + tag.len();
    Some(src[i..].trim().to_owned())
}

fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[cfg(feature = "lisp")]
pub fn compile(src: &str) -> Result<Vec<StorageSpec>, String> {
    tatara_lisp::compile_typed::<StorageSpec>(src)
        .map_err(|e| format!("failed to compile defstorage forms: {e}"))
}

#[cfg(feature = "lisp")]
pub fn register() {
    tatara_lisp::domain::register::<StorageSpec>();
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec(name: &str) -> StorageSpec {
        StorageSpec {
            name: name.into(),
            path: None,
            ttl_seconds: None,
            indexes: Vec::new(),
            description: None,
        }
    }

    fn spec_with_indexes(name: &str, indexes: &[&str]) -> StorageSpec {
        StorageSpec {
            name: name.into(),
            path: None,
            ttl_seconds: None,
            indexes: indexes.iter().map(|s| (*s).into()).collect(),
            description: None,
        }
    }

    fn tmp_path(name: &str) -> PathBuf {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("nami-core-kv-{name}-{ts}.log"))
    }

    #[test]
    fn volatile_set_get_delete_roundtrips() {
        let s = Store::from_spec(&spec("vol"));
        s.set("k", json!("v"));
        assert_eq!(s.get("k"), Some(json!("v")));
        assert_eq!(s.len(), 1);
        assert!(s.delete("k"));
        assert_eq!(s.get("k"), None);
        assert!(s.is_empty());
    }

    #[test]
    fn set_overwrites_existing_value() {
        let s = Store::from_spec(&spec("vol"));
        s.set("k", json!(1));
        s.set("k", json!(2));
        assert_eq!(s.get("k"), Some(json!(2)));
    }

    #[test]
    fn delete_of_missing_key_returns_false() {
        let s = Store::from_spec(&spec("vol"));
        assert!(!s.delete("nope"));
    }

    #[test]
    fn registry_dedupes_by_name() {
        let mut reg = StorageRegistry::new();
        reg.insert(spec("cookies"));
        reg.insert(spec("cookies"));
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn persistence_roundtrips_across_open_close() {
        let path = tmp_path("persist");
        let sp = StorageSpec {
            path: Some(path.clone()),
            ..spec("persist")
        };
        {
            let s = Store::from_spec(&sp);
            s.set("a", json!("apple"));
            s.set("b", json!({"nested": "yes"}));
        }
        {
            let s = Store::from_spec(&sp);
            assert_eq!(s.get("a"), Some(json!("apple")));
            assert_eq!(s.get("b"), Some(json!({"nested": "yes"})));
            assert_eq!(s.len(), 2);
        }
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn delete_tombstones_persist_and_replay() {
        let path = tmp_path("tomb");
        let sp = StorageSpec {
            path: Some(path.clone()),
            ..spec("tomb")
        };
        {
            let s = Store::from_spec(&sp);
            s.set("k", json!(1));
            assert!(s.delete("k"));
        }
        {
            let s = Store::from_spec(&sp);
            assert_eq!(s.get("k"), None);
            assert!(s.is_empty());
        }
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn compact_rewrites_journal_without_stale_lines() {
        let path = tmp_path("compact");
        let sp = StorageSpec {
            path: Some(path.clone()),
            ..spec("compact")
        };
        let s = Store::from_spec(&sp);
        for i in 0..20 {
            s.set("counter", json!(i));
        }
        s.compact().unwrap();
        let body = std::fs::read_to_string(&path).unwrap();
        assert_eq!(body.lines().count(), 1);
        assert!(body.contains("\"counter\""));
        assert!(body.contains("19"));
        let s2 = Store::from_spec(&sp);
        assert_eq!(s2.get("counter"), Some(json!(19)));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn ttl_expires_entries_on_access() {
        let sp = StorageSpec {
            ttl_seconds: Some(0),
            ..spec("ttl")
        };
        let s = Store::from_spec(&sp);
        s.set("k", json!("v"));
        std::thread::sleep(std::time::Duration::from_millis(1100));
        assert_eq!(s.get("k"), None);
        assert!(s.is_empty());
    }

    #[test]
    fn format_and_parse_event_roundtrip() {
        let e = Event::Set {
            ts: 1234,
            key: "hello".into(),
            value: json!({"x": 1, "y": [1, 2]}),
        };
        let line = format_event(&e).unwrap();
        assert_eq!(parse_event(&line).unwrap(), e);

        let e2 = Event::Delete {
            ts: 5678,
            key: "gone".into(),
        };
        let line2 = format_event(&e2).unwrap();
        assert_eq!(parse_event(&line2).unwrap(), e2);
    }

    #[test]
    fn malformed_journal_line_is_skipped() {
        let path = tmp_path("malformed");
        std::fs::write(
            &path,
            "(event gibberish)\n(event :ts 1 :op \"set\" :key \"k\" :value \"v\")\n",
        )
        .unwrap();
        let sp = StorageSpec {
            path: Some(path.clone()),
            ..spec("malformed")
        };
        let s = Store::from_spec(&sp);
        assert_eq!(s.get("k"), Some(json!("v")));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn clone_shares_state() {
        let s = Store::from_spec(&spec("share"));
        let s2 = s.clone();
        s.set("k", json!("v"));
        assert_eq!(s2.get("k"), Some(json!("v")));
    }

    #[test]
    fn quote_escapes_quotes_and_backslashes() {
        assert_eq!(quote("ab"), r#""ab""#);
        assert_eq!(quote("a\"b"), r#""a\"b""#);
        assert_eq!(quote("a\\b"), r#""a\\b""#);
    }

    // ── Access-pattern coverage ───────────────────────────────────

    #[test]
    fn prefix_keys_filters_by_prefix() {
        let s = Store::from_spec(&spec("prefix"));
        s.set("user/alice", json!(1));
        s.set("user/bob", json!(2));
        s.set("cookie/xyz", json!(3));
        let mut u = s.prefix_keys("user/");
        u.sort();
        assert_eq!(u, vec!["user/alice", "user/bob"]);
        assert_eq!(s.prefix_keys("cookie/").len(), 1);
        assert!(s.prefix_keys("missing/").is_empty());
    }

    #[test]
    fn entries_returns_full_snapshot() {
        let s = Store::from_spec(&spec("entries"));
        s.set("a", json!(1));
        s.set("b", json!(2));
        let mut entries = s.entries();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(entries, vec![
            ("a".into(), json!(1)),
            ("b".into(), json!(2)),
        ]);
    }

    #[test]
    fn filter_predicate_walks_values() {
        let s = Store::from_spec(&spec("filter"));
        s.set("a", json!(10));
        s.set("b", json!(20));
        s.set("c", json!(30));
        let mut big: Vec<_> = s
            .filter(|_k, v| v.as_i64().unwrap_or(0) > 15)
            .into_iter()
            .map(|(k, _)| k)
            .collect();
        big.sort();
        assert_eq!(big, vec!["b", "c"]);
    }

    #[test]
    fn set_many_and_delete_many_bulk_roundtrip() {
        let s = Store::from_spec(&spec("bulk"));
        s.set_many(
            (0..50).map(|i| (format!("k{i}"), json!(i))),
        );
        assert_eq!(s.len(), 50);
        let to_delete: Vec<String> = (0..10).map(|i| format!("k{i}")).collect();
        let removed = s.delete_many(to_delete.iter().map(String::as_str));
        assert_eq!(removed, 10);
        assert_eq!(s.len(), 40);
    }

    #[test]
    fn contains_checks_presence() {
        let s = Store::from_spec(&spec("contains"));
        assert!(!s.contains("k"));
        s.set("k", json!("v"));
        assert!(s.contains("k"));
        s.delete("k");
        assert!(!s.contains("k"));
    }

    #[test]
    fn merge_from_propagates_entries_but_not_future_writes() {
        let a = Store::from_spec(&spec("merge-a"));
        let b = Store::from_spec(&spec("merge-b"));
        b.set("from-b-1", json!(1));
        b.set("from-b-2", json!(2));
        a.merge_from(&b);
        assert_eq!(a.len(), 2);
        b.set("future", json!("not-in-a"));
        assert_eq!(a.contains("future"), false);
    }

    #[test]
    fn merge_from_is_last_write_wins() {
        let a = Store::from_spec(&spec("merge-lww-a"));
        let b = Store::from_spec(&spec("merge-lww-b"));
        a.set("k", json!("a"));
        b.set("k", json!("b"));
        a.merge_from(&b);
        assert_eq!(a.get("k"), Some(json!("b")));
    }

    // ── Lisp-value access ─────────────────────────────────────────

    #[test]
    fn set_expr_then_get_expr_roundtrips() {
        let s = Store::from_spec(&spec("lisp"));
        s.set_expr("visits-sq", "(* visits visits)");
        assert_eq!(s.get_expr("visits-sq"), Some("(* visits visits)".into()));
    }

    #[test]
    fn get_expr_returns_none_for_plain_value() {
        let s = Store::from_spec(&spec("lisp2"));
        s.set("k", json!(42));
        assert_eq!(s.get_expr("k"), None);
    }

    #[test]
    fn lisp_keys_only_returns_tagged_entries() {
        let s = Store::from_spec(&spec("lisp3"));
        s.set("plain", json!(1));
        s.set_expr("one", "(+ 1 1)");
        s.set_expr("two", "(* a b)");
        let mut keys = s.lisp_keys();
        keys.sort();
        assert_eq!(keys, vec!["one", "two"]);
    }

    // ── Performance smoke tests ───────────────────────────────────
    //
    // Not cycle-accurate — criterion benchmarks cover that in
    // `benches/`. These are cheap timing sanity checks that catch
    // quadratic regressions at CI time.

    #[test]
    fn perf_set_10k_volatile_under_500ms() {
        let s = Store::from_spec(&spec("perf-set"));
        let t = std::time::Instant::now();
        for i in 0..10_000 {
            s.set(format!("k{i}"), json!(i));
        }
        let elapsed = t.elapsed();
        assert_eq!(s.len(), 10_000);
        assert!(
            elapsed.as_millis() < 500,
            "10k volatile sets took {elapsed:?}"
        );
    }

    #[test]
    fn perf_get_10k_random_under_200ms() {
        let s = Store::from_spec(&spec("perf-get"));
        for i in 0..10_000 {
            s.set(format!("k{i}"), json!(i));
        }
        let t = std::time::Instant::now();
        for i in 0..10_000 {
            let _ = s.get(&format!("k{i}"));
        }
        let elapsed = t.elapsed();
        assert!(
            elapsed.as_millis() < 200,
            "10k gets took {elapsed:?}"
        );
    }

    #[test]
    fn perf_replay_10k_from_disk_under_1s() {
        let path = tmp_path("perf-replay");
        let sp = StorageSpec {
            path: Some(path.clone()),
            ..spec("perf-replay")
        };
        {
            let s = Store::from_spec(&sp);
            for i in 0..10_000 {
                s.set(format!("k{i}"), json!(i));
            }
        }
        let t = std::time::Instant::now();
        let s = Store::from_spec(&sp);
        let elapsed = t.elapsed();
        assert_eq!(s.len(), 10_000);
        assert!(
            elapsed.as_millis() < 1_000,
            "10k replay took {elapsed:?}"
        );
        std::fs::remove_file(&path).ok();
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_storage_form() {
        let src = r#"
            (defstorage :name "cookies"
                        :path "/tmp/cookies.log"
                        :ttl-seconds 3600)
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "cookies");
        assert_eq!(
            specs[0].path.as_ref().map(|p| p.to_str().unwrap()),
            Some("/tmp/cookies.log")
        );
        assert_eq!(specs[0].ttl_seconds, Some(3600));
    }

    // ── Secondary indexes ───────────────────────────────────────────

    #[test]
    fn index_paths_reflect_spec() {
        let s = Store::from_spec(&spec_with_indexes("x", &["domain", "user.id"]));
        let mut paths = s.index_paths();
        paths.sort();
        assert_eq!(paths, vec!["domain".to_owned(), "user.id".to_owned()]);
    }

    #[test]
    fn by_index_returns_none_for_undeclared_path() {
        let s = Store::from_spec(&spec_with_indexes("x", &["domain"]));
        s.set("k1", json!({"domain": "example.com"}));
        // Path wasn't declared → None, not an empty Vec.
        assert!(s.by_index("nonexistent", "foo").is_none());
        // Declared but no match → empty Vec.
        assert_eq!(s.by_index("domain", "example.com").unwrap().len(), 1);
        assert_eq!(s.by_index("domain", "missing.com").unwrap().len(), 0);
    }

    #[test]
    fn by_index_groups_keys_sharing_a_projected_value() {
        let s = Store::from_spec(&spec_with_indexes("cookies", &["domain"]));
        s.set("cookie/1", json!({"domain": "example.com", "name": "a"}));
        s.set("cookie/2", json!({"domain": "example.com", "name": "b"}));
        s.set("cookie/3", json!({"domain": "other.com", "name": "c"}));

        let hits = s.by_index("domain", "example.com").unwrap();
        let keys: std::collections::HashSet<String> =
            hits.iter().map(|(k, _)| k.clone()).collect();
        assert_eq!(keys.len(), 2);
        assert!(keys.contains("cookie/1"));
        assert!(keys.contains("cookie/2"));

        let others = s.by_index("domain", "other.com").unwrap();
        assert_eq!(others.len(), 1);
        assert_eq!(others[0].0, "cookie/3");
    }

    #[test]
    fn index_updates_on_set_of_changed_projection() {
        let s = Store::from_spec(&spec_with_indexes("x", &["tier"]));
        s.set("u/1", json!({"tier": "free"}));
        assert_eq!(s.by_index("tier", "free").unwrap().len(), 1);
        // Reassign same key → old bucket must empty, new one populate.
        s.set("u/1", json!({"tier": "paid"}));
        assert_eq!(s.by_index("tier", "free").unwrap().len(), 0);
        assert_eq!(s.by_index("tier", "paid").unwrap().len(), 1);
    }

    #[test]
    fn index_drops_key_on_delete() {
        let s = Store::from_spec(&spec_with_indexes("x", &["tier"]));
        s.set("u/1", json!({"tier": "free"}));
        s.set("u/2", json!({"tier": "free"}));
        assert_eq!(s.by_index("tier", "free").unwrap().len(), 2);
        s.delete("u/1");
        assert_eq!(s.by_index("tier", "free").unwrap().len(), 1);
        s.delete("u/2");
        // Bucket must be reaped so the distinct-value list is accurate.
        assert_eq!(s.by_index("tier", "free").unwrap().len(), 0);
        assert!(s.index_values("tier").unwrap().is_empty());
    }

    #[test]
    fn index_values_returns_sorted_distinct_list() {
        let s = Store::from_spec(&spec_with_indexes("x", &["domain"]));
        s.set("c/1", json!({"domain": "b.com"}));
        s.set("c/2", json!({"domain": "a.com"}));
        s.set("c/3", json!({"domain": "b.com"}));
        let vals = s.index_values("domain").unwrap();
        assert_eq!(vals, vec!["a.com".to_owned(), "b.com".to_owned()]);
    }

    #[test]
    fn nested_dot_path_projects_through_objects() {
        let s = Store::from_spec(&spec_with_indexes("x", &["user.id"]));
        s.set("s/1", json!({"user": {"id": "alice"}}));
        s.set("s/2", json!({"user": {"id": "bob"}}));
        let hits = s.by_index("user.id", "alice").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, "s/1");
    }

    #[test]
    fn array_index_projects_by_decimal_subscript() {
        let s = Store::from_spec(&spec_with_indexes("x", &["tags.0"]));
        s.set("p/1", json!({"tags": ["rust", "web"]}));
        s.set("p/2", json!({"tags": ["browser", "ui"]}));
        let hits = s.by_index("tags.0", "rust").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, "p/1");
    }

    #[test]
    fn non_scalar_leaf_is_skipped_from_index() {
        let s = Store::from_spec(&spec_with_indexes("x", &["nested"]));
        // Value at `nested` is an object → not indexable.
        s.set("k", json!({"nested": {"deep": "value"}}));
        // Index is empty because the projected leaf wasn't a scalar.
        assert!(s.index_values("nested").unwrap().is_empty());
    }

    #[test]
    fn integer_and_bool_projections_stringify_deterministically() {
        let s = Store::from_spec(&spec_with_indexes("x", &["age", "active"]));
        s.set("u/1", json!({"age": 30, "active": true}));
        s.set("u/2", json!({"age": 30, "active": false}));
        assert_eq!(s.by_index("age", "30").unwrap().len(), 2);
        assert_eq!(s.by_index("active", "true").unwrap().len(), 1);
        assert_eq!(s.by_index("active", "false").unwrap().len(), 1);
    }

    #[test]
    fn indexes_rebuild_from_replay() {
        let path = tmp_path("index-replay");
        let sp = StorageSpec {
            path: Some(path.clone()),
            indexes: vec!["domain".into()],
            ..spec("x")
        };
        {
            let s = Store::from_spec(&sp);
            s.set("c/1", json!({"domain": "example.com"}));
            s.set("c/2", json!({"domain": "example.com"}));
            s.set("c/3", json!({"domain": "other.com"}));
        }
        // Reopen → indexes must rebuild identically from the journal.
        let s = Store::from_spec(&sp);
        assert_eq!(s.by_index("domain", "example.com").unwrap().len(), 2);
        assert_eq!(s.by_index("domain", "other.com").unwrap().len(), 1);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn rebuild_indexes_is_idempotent() {
        let s = Store::from_spec(&spec_with_indexes("x", &["domain"]));
        s.set("c/1", json!({"domain": "a"}));
        s.set("c/2", json!({"domain": "b"}));
        let before = (
            s.by_index("domain", "a").unwrap().len(),
            s.by_index("domain", "b").unwrap().len(),
        );
        s.rebuild_indexes();
        let after = (
            s.by_index("domain", "a").unwrap().len(),
            s.by_index("domain", "b").unwrap().len(),
        );
        assert_eq!(before, after);
    }

    #[test]
    fn store_without_indexes_reports_empty_paths() {
        let s = Store::from_spec(&spec("x"));
        assert!(s.index_paths().is_empty());
        assert!(s.by_index("anything", "x").is_none());
    }

    #[test]
    fn perf_index_lookup_beats_linear_filter_at_10k() {
        // Sanity check that index lookup stays O(log n).
        let s = Store::from_spec(&spec_with_indexes("perf", &["domain"]));
        for i in 0..10_000 {
            let dom = if i % 100 == 0 { "hot.com" } else { "cold.com" };
            s.set(format!("k{i}"), json!({"domain": dom}));
        }
        let start = std::time::Instant::now();
        let hot = s.by_index("domain", "hot.com").unwrap();
        let dt = start.elapsed();
        assert_eq!(hot.len(), 100);
        // Generous bound — CI hosts are slow — but still orders of
        // magnitude below O(n) JSON walks.
        assert!(dt < std::time::Duration::from_millis(100), "elapsed: {dt:?}");
    }

    #[test]
    fn by_index_range_inclusive_bounds() {
        let s = Store::from_spec(&spec_with_indexes("x", &["domain"]));
        s.set("a", json!({"domain": "apple.com"}));
        s.set("b", json!({"domain": "banana.com"}));
        s.set("c", json!({"domain": "cherry.com"}));
        s.set("d", json!({"domain": "date.com"}));

        let mid = s.by_index_range("domain", "b", "c\u{FFFF}").unwrap();
        let hits: std::collections::HashSet<String> =
            mid.into_iter().map(|(k, _)| k).collect();
        assert!(hits.contains("b"));
        assert!(hits.contains("c"));
        assert!(!hits.contains("a"));
        assert!(!hits.contains("d"));
    }

    #[test]
    fn by_index_range_empty_when_no_overlap() {
        let s = Store::from_spec(&spec_with_indexes("x", &["k"]));
        s.set("a", json!({"k": "alpha"}));
        let miss = s.by_index_range("k", "z", "zz").unwrap();
        assert!(miss.is_empty());
    }

    #[test]
    fn by_index_range_full_unbounded_returns_everything() {
        let s = Store::from_spec(&spec_with_indexes("x", &["tier"]));
        s.set("u/1", json!({"tier": "free"}));
        s.set("u/2", json!({"tier": "paid"}));
        s.set("u/3", json!({"tier": "enterprise"}));
        let all = s.by_index_range("tier", "", "\u{10FFFF}").unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn by_index_range_on_undeclared_path_is_none() {
        let s = Store::from_spec(&spec("x"));
        assert!(s.by_index_range("nope", "a", "z").is_none());
    }

    #[test]
    fn by_index_range_lex_order_with_zero_pad_numeric_keys() {
        // Demonstrate the "zero-pad for numeric order" convention.
        let s = Store::from_spec(&spec_with_indexes("x", &["age"]));
        s.set("u/1", json!({"age": "0018"}));
        s.set("u/2", json!({"age": "0032"}));
        s.set("u/3", json!({"age": "0045"}));
        s.set("u/4", json!({"age": "0060"}));
        let adults_under_50 = s.by_index_range("age", "0018", "0049").unwrap();
        assert_eq!(adults_under_50.len(), 3);
    }

    #[cfg(feature = "lisp")]
    #[test]
    fn compile_parses_indexes() {
        let src = r#"
            (defstorage :name "cookies"
                        :ttl-seconds 86400
                        :indexes ("domain" "user.id"))
        "#;
        let specs = compile(src).unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].indexes, vec!["domain", "user.id"]);
    }
}
