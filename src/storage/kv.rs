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
        Self {
            inner: Arc::new(Mutex::new(StoreInner {
                name: spec.name.clone(),
                path: spec.path.clone(),
                ttl: spec.ttl_seconds,
                entries,
            })),
        }
    }

    pub fn set(&self, key: impl Into<String>, value: Value) {
        let now = now_secs();
        let key = key.into();
        let mut inner = self.lock();
        inner.entries.insert(
            key.clone(),
            Entry {
                value: value.clone(),
                ts: now,
            },
        );
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

    /// Live key snapshot — excludes TTL-expired entries.
    #[must_use]
    pub fn keys(&self) -> Vec<String> {
        let mut inner = self.lock();
        let ttl = inner.ttl;
        let now = now_secs();
        if let Some(t) = ttl {
            inner.entries.retain(|_, e| now.saturating_sub(e.ts) <= t);
        }
        inner.entries.keys().cloned().collect()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.keys().len()
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
}
