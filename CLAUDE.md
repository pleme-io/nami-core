# nami-core -- Shared Browser Core Library

> **★★★ CSE / Knowable Construction.** This repo operates under **Constructive Substrate Engineering** — canonical specification at [`pleme-io/theory/CONSTRUCTIVE-SUBSTRATE-ENGINEERING.md`](https://github.com/pleme-io/theory/blob/main/CONSTRUCTIVE-SUBSTRATE-ENGINEERING.md). The Compounding Directive (operational rules: solve once, load-bearing fixes only, idiom-first, models stay current, direction beats velocity) is in the org-level pleme-io/CLAUDE.md ★★★ section. Read both before non-trivial changes.


Rendering-agnostic browser engine core shared by aranami (TUI/GPU browser) and
namimado (desktop browser with Servo). Provides the full content pipeline from
raw HTML to positioned layout boxes, plus browser infrastructure (bookmarks,
history, content blocking, configuration).

Crate name: `nami-core`. Published to crates.io.

## Build & Test

```bash
cargo build                         # default (no network, no config)
cargo build --features network      # with todoku HTTP client
cargo build --features config       # with shikumi config integration
cargo build --all-features          # everything
cargo test                          # all tests (55 total)
cargo test --lib                    # unit tests only
cargo clippy -- -W clippy::pedantic # lint check
nix build                           # Nix package
nix run .#check-all                 # full CI check (clippy, test, fmt)
nix run .#regenerate                # regenerate Cargo.nix after dep changes
```

After any `Cargo.toml` change, regenerate Cargo.nix:
```bash
nix run github:nix-community/crate2nix -- generate
```

## Competitive Position

| vs | nami-core advantage |
|----|---------------------|
| **Servo** | Embeddable library (not a full browser), tiny footprint (~5MB), clean Rust API, no compositor/embedding complexity |
| **WebKit/Blink** | Pure Rust (no C++), deterministic Nix builds, no IPC/multi-process overhead, scriptable via Rhai |
| **Gecko** | No XUL/XPCOM baggage, single-crate dependency, no SpiderMonkey requirement |
| **Ladybird (LibWeb)** | Rust not C++, uses battle-tested html5ever/lightningcss/taffy rather than from-scratch parsers |
| **scraper/select.rs** | Full CSS cascade + layout computation, not just DOM extraction |
| **lol-html** | Produces a layouted tree (not streaming rewriter), CSS cascade, bidirectional queries |

### Why three crates instead of a full engine?

html5ever (HTML parsing), lightningcss (CSS parsing + cascade), and taffy
(flexbox/grid layout) are each best-in-class for their domain. Combining them
gives spec-compliant behavior without building a monolithic engine. The
combination is lightweight, fully Rust-native, and lets consumers choose their
own rendering backend (garasu GPU, Servo, terminal, headless).

## Architecture

### Content Pipeline

```
HTML source
    |
    v
dom::Document::parse()        <-- html5ever (spec-compliant HTML5 parsing)
    |
    v
DOM tree (dom::Node)           <-- tree traversal, queries, text extraction
    |                               query_selector / query_selector_all
    v
css::StyleSheet::parse()       <-- lightningcss (CSS parsing)
    |
    v
css::StyleResolver::resolve()  <-- cascade: match rules to DOM elements
    |                               block/inline defaults, specificity
    v
StyledTree (css::StyledNode)   <-- DOM + computed CSS properties
    |
    v
layout::LayoutEngine::compute()  <-- taffy (flexbox/grid layout)
    |
    v
LayoutTree (layout::LayoutBox)   <-- absolute x/y/width/height for each box
    |                                 hit testing, tree traversal
    v
Consumer renders (garasu, Servo, terminal, headless...)
```

### Module Map

| Module | Files | Lines | Tests | Purpose |
|--------|-------|-------|-------|---------|
| `dom` | `mod.rs`, `node.rs`, `tree.rs` | ~700 | 10 | HTML5 parsing (html5ever), DOM tree, node traversal, queries, link extraction |
| `css` | `mod.rs`, `cascade.rs` | ~389 | 6 | CSS parsing (lightningcss), cascade resolution, computed styles, block element defaults |
| `layout` | `mod.rs`, `engine.rs` | ~393 | 5 | Flexbox/grid layout (taffy), viewport-relative positioning, hit testing |
| `net` | `mod.rs`, `fetch.rs` | ~292 | 6 | HTTP types, URL resolution; stub without `network` feature, todoku with it |
| `content` | `mod.rs`, `blocker.rs` | ~340 | 6 | Ad/tracker blocking, EasyList-compatible filter lists, domain + pattern matching |
| `storage` | `mod.rs`, `bookmarks.rs`, `history.rs` | ~827 | 17 | Bookmark CRUD with tags/folders, browsing history with visit counting and eviction |
| `config.rs` | (single file) | ~281 | 5 | `BrowserConfig` (homepage, search engine, content blocking, privacy, network, storage) |

### Key Types

**DOM layer:**
- `Document` -- Parsed HTML document. Entry point: `Document::parse(html)`.
  Methods: `query_selector`, `query_selector_all`, `title`, `links`, `text_content`.
- `Node` -- DOM tree node. Variants: Document, Element, Text, Comment.
  Methods: `descendants()` (depth-first iter), `text_content()`, `as_element()`.
- `ElementData` -- Tag name + attributes. Methods: `get_attribute`, `has_class`, `id`.

**CSS layer:**
- `StyleSheet` -- Parsed CSS rules. Entry: `StyleSheet::parse(css)`.
- `StyleResolver` -- Matches rules to DOM elements. `add_sheet()` then `resolve(&doc)`.
- `ComputedStyle` -- Property map for a single element. `get("color")`, `display()`.
- `StyledTree` / `StyledNode` -- DOM + computed styles, ready for layout.

**Layout layer:**
- `LayoutEngine` -- Wraps taffy. `compute(&styled_tree, viewport_size)` -> `LayoutTree`.
- `LayoutBox` -- Positioned box with absolute x/y/width/height.
  Methods: `contains_point`, `hit_test` (deepest box at a point).
- `Size` -- Viewport dimensions (width, height in pixels).

**Content blocking:**
- `ContentBlocker` -- Filter engine. `load_filter_list(text)`, `should_block(url, type)`,
  `record_decision()` (with stats). EasyList subset: `||domain^`, `@@||domain^`, patterns.
- `ResourceType` -- Document, Stylesheet, Script, Image, Font, XHR, Media, WebSocket, etc.

**Storage:**
- `BookmarkStore` -- JSON-backed bookmarks. CRUD, search, tags, folders. `in_memory()` for tests.
- `HistoryStore` -- JSON-backed history. Visit counting, search, eviction, domain stats.

**Config:**
- `BrowserConfig` -- Top-level config with sub-configs for content blocking, privacy, network, storage.
  `search_url(query)` builds a search URL. `discover_path()` finds XDG config file.

---

## Feature Gates

| Feature | Enables | Optional Deps |
|---------|---------|---------------|
| `default` | DOM, CSS, layout, content, storage, config types | None |
| `network` | HTTP fetch via todoku (authenticated, retry, timeout) | `todoku` |
| `config` | shikumi config integration (file discovery, hot-reload) | `shikumi` |

The default feature set has zero optional dependencies -- consumers that bring
their own HTTP client and config loading only need the core pipeline.

---

## How Consumers Use nami-core

### aranami (TUI/GPU browser)

Aranami uses the **full pipeline**. It owns rendering via garasu (GPU) and
mojiban (rich text):

```
nami-core::dom::Document::parse(html)
    -> nami-core::css::StyleResolver::resolve(&doc)
        -> nami-core::layout::LayoutEngine::compute(&styled, viewport)
            -> Walk LayoutTree, emit garasu draw commands
                (text spans via mojiban, images via image crate,
                 rects for backgrounds/borders)
```

Aranami also uses:
- `nami-core::content::ContentBlocker` for ad/tracker blocking
- `nami-core::storage::{BookmarkStore, HistoryStore}` for bookmarks and history
- `nami-core::config::BrowserConfig` as the base config, extended with aranami-specific fields (appearance, keybindings)
- `nami-core::net::FetchClient` for URL resolution (HTTP fetch via `network` feature + todoku)

Aranami adds on top of nami-core:
- GPU rendering (garasu + mojiban + egaku for browser chrome)
- Vim-style keyboard navigation (awase)
- Rhai scripting (soushi) -- wired to nami-core content transform hooks
- Embedded MCP server (kaname) -- backed by nami-core DOM/storage operations
- Daemon mode (tsunagu) for background prefetch

### namimado (desktop browser with Servo)

Namimado delegates web content rendering to Servo, so it does **NOT** use the
DOM/CSS/layout pipeline for page rendering. Instead it uses nami-core for
browser infrastructure only:

```
nami-core::storage::BookmarkStore   -- shared bookmarks (same format as aranami)
nami-core::storage::HistoryStore    -- shared browsing history
nami-core::content::ContentBlocker  -- content blocking rules (fed to Servo as block list)
nami-core::config::BrowserConfig    -- base config (extended with namimado-specific fields)
```

Namimado adds on top of nami-core:
- Servo web engine for full HTML/CSS/JS rendering
- GPU chrome (garasu + egaku + irodzuki) for tabs, toolbar, sidebar
- Browser-standard + vim-style shortcuts (awase)
- Rhai scripting (soushi) -- wired to Servo APIs + nami-core storage
- Embedded MCP server (kaname) -- includes `evaluate_js` via Servo
- IPC bridge between Rust chrome and Servo content

This split ensures both browsers share identical bookmarks, history, content
blocking rules, and base configuration. A bookmark saved in aranami appears
in namimado and vice versa (both read the same JSON file).

---

## Shared Library Integration

| Library | Feature | Used For |
|---------|---------|----------|
| **shikumi** | `config` | Config file discovery, hot-reload via ArcSwap, YAML loading |
| **todoku** | `network` | Authenticated HTTP fetch with retry, timeout, redirects |

nami-core intentionally has minimal pleme-io dependencies. It is a pure content
library -- rendering (garasu, mojiban), UI (egaku), theming (irodzuki), scripting
(soushi), and MCP (kaname) belong in the consumer applications, not here.

---

## Plugin System via soushi (Rhai)

nami-core does not depend on soushi directly. Instead, it exposes content
transform APIs that consumers wire to the soushi Rhai scripting engine.
Scripts live in `~/.config/{app}/scripts/*.rhai`.

### Content Transform Hooks

Consumers register these hooks with soushi. nami-core provides the types and
operations; the consumer bridges them to the Rhai API.

**Page content filters** (run after DOM parse):
```
fn on_page_loaded(doc: Document) -> Document
```
Use cases:
- Reader mode: strip navigation/ads/sidebars, reformat for readability
- Dark mode injection: insert CSS overrides into `<head>`
- Custom content extraction pipelines (save article text, collect links)
- Domain-specific transforms (remove paywalls, expand collapsed content)

**Content blocker extensions** (extend `ContentBlocker`):
```
fn should_block(url: String, resource_type: String) -> bool
```
Use cases:
- Custom blocking logic beyond EasyList patterns
- Domain-specific content policies
- Allowlisting for specific workflows

**Storage hooks**:
```
nami.bookmark(url, tags)         // add bookmark via BookmarkStore
nami.bookmarks_search(query)     // search bookmarks
nami.history_search(query)       // search history
nami.history_recent(limit)       // get recent history
```

**Event hooks** (consumer-defined, backed by nami-core operations):
- `on_navigate(url)` -- before navigation, can modify/block URL
- `on_page_load(doc)` -- after DOM parse, can transform document
- `on_link_hover(url)` -- link hover preview
- `on_blocked_request(url, count)` -- content blocker notification
- `on_download_start(url)` -- download initiated
- `on_download_complete(path)` -- download finished

### Example Rhai Scripts

**Reader mode** (`reader.rhai`):
```rhai
fn on_page_load(doc) {
    let article = doc.query_selector("article");
    if article != () {
        // Extract article content, strip everything else
        nami.set_content(article.text_content());
    }
}
```

**Auto-bookmarker** (`auto_bookmark.rhai`):
```rhai
fn on_page_load(doc) {
    let domain = nami.url().domain();
    if domain == "docs.rs" {
        nami.bookmark(nami.url(), ["docs", "rust"]);
    }
}
```

**Custom blocker** (`block_social.rhai`):
```rhai
fn should_block(url, resource_type) {
    let social = ["facebook.com", "twitter.com", "tiktok.com"];
    for domain in social {
        if url.contains(domain) { return true; }
    }
    false
}
```

---

## MCP Server Integration via kaname

nami-core does not embed a kaname MCP server -- consumers do. nami-core
provides the backing operations for browser-specific MCP tools.

### Operations nami-core provides for MCP tools

| MCP Tool | nami-core Operation | Notes |
|----------|---------------------|-------|
| `navigate` | `FetchClient::fetch(url)` + `Document::parse()` | Requires `network` feature |
| `get_dom` | `Document::query_selector_all(sel)` serialized as JSON | Full tree or subtree |
| `get_links` | `Document::links()` | Returns `Vec<(href, text)>` |
| `extract_text` | `Node::text_content()` via `query_selector()` | CSS selector targeting |
| `search_text` | Text search over `Document::text_content()` | Substring match |
| `get_page_source` | Raw HTML stored by consumer before parsing | Consumer responsibility |
| `get_title` | `Document::title()` | `<title>` tag extraction |
| `bookmark_add` | `BookmarkStore::add()` | With tags and folder |
| `bookmark_list` | `BookmarkStore::search()` or `BookmarkStore::all()` | Full-text search |
| `history_search` | `HistoryStore::search()` | URL and title matching |
| `history_recent` | `HistoryStore::recent(limit)` | Most recent visits |
| `content_block_stats` | `ContentBlocker::stats()` | Checked/blocked/allowed counts |
| `block_domain` | `ContentBlocker::block_domain()` | Runtime domain blocking |

### Consumer MCP tool examples

**aranami** exposes these tools via kaname (stdio transport):
- All nami-core-backed tools above
- `screenshot` -- capture GPU viewport as PNG (garasu operation)
- `click_link` -- follow link by index (aranami navigation)
- `scroll` -- scroll viewport (aranami render)

**namimado** exposes additional tools:
- All nami-core-backed tools above
- `evaluate_js` -- execute JavaScript via Servo (namimado-specific)
- `screenshot` -- full page or viewport capture via Servo
- `tab_list`, `tab_new`, `tab_close`, `tab_switch` -- tab management
- `devtools_open` -- Servo DevTools integration
- `network_requests` -- Servo network log

---

## Configuration via shikumi

### BrowserConfig Structure

The `BrowserConfig` type is the shared base configuration. Consumers extend it
with app-specific fields.

```yaml
# ~/.config/nami/nami.yaml (or per-consumer: ~/.config/nami/nami.yaml, ~/.config/namimado/namimado.yaml)
homepage: "about:blank"
search_engine: "https://duckduckgo.com/?q={}"
content_blocking:
  enabled: true
  filter_lists: []                    # paths to EasyList-format files
  extra_blocked_domains: []           # additional domains to block
  allowed_domains: []                 # exception domains
privacy:
  do_not_track: true
  block_third_party_cookies: true
  https_only: false
  clear_on_exit: false
network:
  timeout_ms: 30000
  max_connections: 6
  user_agent: ""                      # empty = default nami-core UA
  max_redirects: 10
storage:
  max_history_entries: 10000
  bookmarks_file: "bookmarks.json"
  history_file: "history.json"
```

### Config discovery

- `BrowserConfig::discover_path()` checks `$XDG_CONFIG_HOME/nami/nami.yaml`
  then `$XDG_CONFIG_HOME/nami/nami.json`
- Environment override: `NAMI_CONFIG=/path/to/config.yaml` (with `config` feature)
- Environment prefix: `NAMI_` (e.g., `NAMI_HOMEPAGE=https://example.com`)
- Hot-reload: shikumi ArcSwap + file watcher (with `config` feature)

### Consumer config extensions

**aranami** extends `BrowserConfig` with:
```yaml
appearance:
  font_size: 14
  images: true
  dark_mode: true
  link_color: "#88c0d0"
keybindings: {}
```

**namimado** extends `BrowserConfig` with:
```yaml
theme:
  dark: true
  font_size: 14.0
  toolbar_opacity: 1.0
devtools_enabled: false
sidebar:
  visible: true
  position: "left"
```

---

## Module Details

### dom/ -- HTML Parsing and DOM Tree

**Parser**: html5ever (Mozilla's spec-compliant HTML5 parser). Handles malformed
HTML correctly via the spec's error recovery algorithm.

**Core types**:

| Type | Description |
|------|-------------|
| `Document` | Root container. Created via `Document::parse(html)`. Owns the root `Node`. |
| `Node` | Tree node with `data: NodeData` and `children: Vec<Node>`. Methods: `is_element()`, `is_text()`, `as_element()`, `as_text()`, `text_content()`, `descendants()`, `append_child()` |
| `NodeData` | Enum: `Document`, `Element(ElementData)`, `Text(String)`, `Comment(String)` |
| `ElementData` | Tag name + attributes. Methods: `get_attribute(name)`, `has_class(name)`, `id()` |
| `DescendantIter` | Depth-first iterator over all descendant nodes |

**Query API** (on `Document`):
- `query_selector(selector)` -- First element matching a simple CSS selector (tag, `.class`, `#id`)
- `query_selector_all(selector)` -- All elements matching a simple CSS selector
- `title()` -- Text content of `<title>` element
- `links()` -- All `<a href="...">` as `(href, text)` pairs

**Design note**: The DOM uses an owned-tree model (`Vec<Node>` children) rather
than arena allocation. This simplifies the API at the cost of O(n) cloning.
For browser-scale DOMs, migration to an arena (e.g., `typed-arena` or
`indextree`) would improve performance. Not needed for current use cases.

**TreeSink note**: html5ever's `TreeSink` trait passes `&self` to methods that
need mutation (`create_element`, `append`). The implementation uses
`#[expect(invalid_reference_casting)]` with unsafe pointer casts, following the
same pattern as other html5ever consumers (kuchiki, markup5ever_rcdom). A future
refactor could use `RefCell` or `UnsafeCell` for interior mutability.

### css/ -- CSS Parsing and Cascade

**Parser**: lightningcss (Parcel team's CSS parser/transformer). Fast, handles
modern CSS, good error recovery.

**Core types**:

| Type | Description |
|------|-------------|
| `StyleSheet` | Parsed CSS. Contains `Vec<StyleRule>`. Created via `StyleSheet::parse(css)`. |
| `StyleRule` | Selector string + `Vec<Declaration>` |
| `Declaration` | Property name + value (both strings) |
| `StyleResolver` | Collects stylesheets, resolves styles against a DOM tree. `add_sheet()`, `resolve(document)` |
| `StyledTree` | Tree of `StyledNode` values with computed styles |
| `StyledNode` | Tag name + `ComputedStyle` + children |
| `ComputedStyle` | `HashMap<String, String>` of resolved property values. Methods: `get(prop)`, `set(prop, val)`, `display()` |

**Cascade implementation** (current):
- Block elements get `display: block` by default (recognized tags: div, p, h1-h6,
  ul, ol, li, blockquote, pre, section, article, header, footer, nav, main, form,
  table, figure, figcaption, details, summary)
- Rules applied in stylesheet order (specificity not yet fully implemented)
- Simple selector matching (tag name matching against lightningcss selector debug output)

**Property extraction**: Known CSS properties (color, background-color, display,
width, height, margin-*, padding-*, font-size, font-family) are extracted with
clean names and formatted values. Unknown properties fall back to Debug formatting.

**Color formatting**: RGBA colors formatted as `#rrggbb` (opaque) or
`rgba(r, g, b, a)` (translucent). `currentColor` preserved.

**Cascade improvements needed** (ordered by impact):
1. Full selector matching (class, ID, attribute, combinator) instead of substring matching on debug output
2. Specificity calculation and correct ordering
3. Property inheritance (color, font-*) from parent to child
4. Shorthand expansion (margin, padding, border, font)
5. Computed values (em, rem, %, vh/vw resolution)
6. Default UA stylesheet

### layout/ -- Layout Computation

**Engine**: taffy (pure Rust flexbox + grid layout engine). Computes positions
and dimensions for all elements in the styled tree.

**Core types**:

| Type | Description |
|------|-------------|
| `LayoutEngine` | Wraps `TaffyTree`. Methods: `compute(styled_tree, viewport) -> LayoutTree` |
| `LayoutTree` | Root `LayoutBox` + viewport `Size` |
| `LayoutBox` | `x, y, width, height, tag, node_index, children`. Methods: `contains_point()`, `hit_test()` |
| `Size` | `width: f32, height: f32` |

**Style-to-taffy mapping**:
- `display`: block, flex, none (inline treated as block since taffy lacks inline)
- `width`, `height`: pixel values parsed from strings
- `margin-*`: top/right/bottom/left pixel values
- `padding-*`: top/right/bottom/left pixel values

**Layout improvements needed** (ordered by impact):
1. Inline layout (taffy lacks inline flow -- supplement with custom pass or text layout engine)
2. Text measurement (feed actual text dimensions to taffy; currently zero intrinsic size)
3. Percentage units (resolve `%` against parent dimensions)
4. Auto margins (`margin: auto` for centering)
5. Table layout algorithm
6. Positioned elements (`position: absolute/fixed/relative`)
7. Float layout

### content/ -- Content Blocking

**Engine**: Domain-based and pattern-based blocking with EasyList-compatible
filter list parsing.

**Core types**:

| Type | Description |
|------|-------------|
| `ContentBlocker` | Main blocker. Methods: `block_domain()`, `allow_domain()`, `should_block()`, `record_decision()`, `load_filter_list()` |
| `ResourceType` | Enum: Document, Stylesheet, Script, Image, Font, XmlHttpRequest, Media, WebSocket, SubDocument, Other |
| `BlockerStats` | `checked`, `blocked`, `allowed` counters |

**Filter list format** (EasyList subset):
- `||domain.com^` -- block domain and all subdomains
- `@@||domain.com^` -- exception (allow) rule
- Any other non-comment line -- substring pattern match against URL
- `!` or `[` prefix -- comment/header (ignored)

**Blocking logic**:
1. Check if blocker is enabled (disabled = allow all)
2. Check allow list (exceptions) first -- if matched, allow
3. Check blocked domains (with subdomain matching) -- if matched, block
4. Check blocked patterns (substring match) -- if matched, block
5. Otherwise, allow

### storage/ -- Bookmarks and History

**Bookmarks** (`BookmarkStore`):
- JSON file persistence
- CRUD: `add()`, `remove()`, `get()`, `contains()`
- Search: `search(query)` matches URL, title, and tags
- Organization: `in_folder(name)`, `with_tag(tag)`, `all_tags()`
- Deduplication: rejects duplicate URLs
- Builder pattern: `Bookmark::new(url, title).with_tags(vec).in_folder(name)`
- `in_memory()` constructor for tests

**History** (`HistoryStore`):
- JSON file persistence
- Deduplication by URL with visit count increment
- `record(url, title)` -- add or update entry
- `search(query)` -- match URL and title, sorted by recency
- `recent(limit)` -- most recently visited
- `most_visited(limit)` -- highest visit count
- `domain_stats()` -- per-domain visit counts
- Auto-eviction: oldest entries removed when exceeding `max_entries` (default 10,000)
- `in_memory()` constructor for tests

### net/ -- HTTP Fetch

Behind the `network` feature flag. Without the feature, provides types and a
stub `FetchClient::fetch()` that returns an error.

**Core types**:

| Type | Description |
|------|-------------|
| `FetchClient` | HTTP client. `fetch(url)`, `resolve_url(base, href)`, `response_from_parts()` |
| `FetchConfig` | User-agent, timeout, max redirects, cookie acceptance |
| `Response` | Status, headers, body, final URL. Methods: `text()`, `content_type()`, `is_success()`, `is_html()`, `is_css()` |
| `FetchError` | InvalidUrl, NetworkError, HttpError, Blocked, Timeout |

Consumers that bring their own HTTP client can construct `Response` values
via `FetchClient::response_from_parts()` and use only the DOM/CSS/layout
pipeline.

### config.rs -- Browser Configuration

Available with or without the `config` feature. Without shikumi, provides
the `BrowserConfig` struct with `from_json()` and `discover_path()`.
With shikumi, adds hot-reload via ArcSwap and YAML file loading.

**Sub-configs**: `ContentBlockingConfig`, `PrivacyConfig`, `NetworkConfig`, `StorageConfig`.

---

## API Surface for Consumers

### Minimal usage (DOM + CSS + layout)

```rust
use nami_core::dom::Document;
use nami_core::css::{StyleSheet, StyleResolver};
use nami_core::layout::{LayoutEngine, Size};

// Parse HTML
let doc = Document::parse("<div><p>Hello</p></div>");

// Parse and resolve CSS
let sheet = StyleSheet::parse("p { color: red; display: block; }")?;
let mut resolver = StyleResolver::new();
resolver.add_sheet(sheet);
let styled = resolver.resolve(&doc);

// Compute layout
let mut engine = LayoutEngine::new();
let layout = engine.compute(&styled, Size::new(800.0, 600.0));

// Use layout boxes for rendering
// layout.root.x, layout.root.y, layout.root.width, layout.root.height
// layout.root.children[0] ...
// layout.root.hit_test(px, py) -> deepest box at point
```

### Content blocking

```rust
use nami_core::content::{ContentBlocker, ResourceType};

let mut blocker = ContentBlocker::new();
blocker.load_filter_list(easylist_content)?;

let url = url::Url::parse("https://ads.example.com/tracker.js")?;
if blocker.record_decision(&url, ResourceType::Script) {
    // blocked -- skip this request
}

println!("blocked {}/{} requests", blocker.stats().blocked, blocker.stats().checked);
```

### Bookmarks and history

```rust
use nami_core::storage::{Bookmark, BookmarkStore, HistoryStore};

let mut bookmarks = BookmarkStore::new(path)?;
bookmarks.add(Bookmark::new("https://rust-lang.org", "Rust").with_tags(vec!["lang".into()]))?;
let results = bookmarks.search("rust");
bookmarks.save()?;

let mut history = HistoryStore::new(path)?;
history.record("https://example.com", "Example Site");
let recent = history.recent(10);
let top = history.most_visited(5);
let stats = history.domain_stats();
history.save()?;
```

---

## Dependencies

| Crate | Version | Role |
|-------|---------|------|
| `html5ever` | 0.29 | HTML5 parsing (spec-compliant, error recovery) |
| `markup5ever` | 0.14 | Shared types for html5ever (QualName, etc.) |
| `lightningcss` | 1.0.0-alpha | CSS parsing and property extraction |
| `taffy` | 0.7 | Flexbox and grid layout computation |
| `url` | 2 | URL parsing and manipulation |
| `serde` | 1 | Serialization for config, bookmarks, history |
| `serde_json` | 1 | JSON persistence for storage modules |
| `tracing` | 0.1 | Structured logging |
| `thiserror` | 2 | Error type derivation |
| `todoku` | git (optional) | HTTP client (behind `network` feature) |
| `shikumi` | git (optional) | Config discovery and hot-reload (behind `config` feature) |

---

## Design Decisions

### Why a separate library (not inline in each browser)?

The DOM parsing, CSS cascade, layout computation, bookmark/history storage, and
content blocking are identical between aranami and namimado. Extracting them
into nami-core eliminates ~3000 lines of duplication and ensures both browsers
behave identically for content operations.

### Why rendering-agnostic?

nami-core produces a `LayoutTree` of positioned boxes. It does not know how to
draw them. This lets aranami render via garasu (GPU), namimado delegate to Servo,
and future consumers render to terminal, PDF, or headless test harnesses.

### Why html5ever + lightningcss + taffy?

These three crates provide spec-compliant HTML parsing, fast CSS handling, and
correct flexbox/grid layout without pulling in a full browser engine. Each is
the best available Rust crate for its domain. The combination is lightweight
and fully Rust-native.

### Why feature gates for network and config?

Consumers that bring their own HTTP client (e.g., namimado uses Servo's
networking) should not be forced to pull in todoku. The `network` feature is
opt-in. Similarly, consumers with their own config system can skip shikumi.

### Why no JavaScript engine?

JavaScript execution is the most complex part of a browser engine. nami-core
focuses on the content pipeline (HTML + CSS + layout) and gets that right first.
JavaScript can be added later via boa_engine behind a feature flag without
changing the existing API.

### Why owned-tree DOM (not arena)?

The owned-tree model (`Node { children: Vec<Node> }`) is simpler to implement
and reason about. Arena allocation (indextree, typed-arena) would improve
performance for large DOMs but adds API complexity. The current model is correct
and fast enough for initial use cases. Migration is planned for Phase 6.

### Why HashMap for ComputedStyle (not typed struct)?

A typed struct would be more efficient and catch property name typos at compile
time. The HashMap approach was chosen for iteration speed during development --
adding a new CSS property does not require changing the ComputedStyle struct.
Once the set of supported properties stabilizes, migration to a typed struct
with an `Other(HashMap)` fallback is planned.

### Why JSON persistence for storage (not SQLite)?

JSON files are simpler, human-readable, and do not require a SQLite dependency.
For the expected scale (10,000 history entries, hundreds of bookmarks), JSON is
fast enough. Migration to SQLite or redb is straightforward since the store APIs
are already abstracted.

### Why lightningcss (not cssparser)?

lightningcss builds on cssparser (same author) but provides higher-level
property types. It can parse `Property::Color(CssColor::RGBA(...))` directly
instead of requiring manual property parsing. It also handles browser prefixes
and modern CSS features.

### Why taffy (not custom layout)?

CSS layout is extremely complex (the flexbox spec alone is 100+ pages). taffy
implements both flexbox and grid layout correctly. Writing custom layout code
would take months and have correctness issues. The tradeoff is that taffy lacks
inline flow layout, which needs to be supplemented.

### Why the TreeSink unsafe casts?

html5ever's `TreeSink` trait passes `&self` to methods that need mutation
(`create_element`, `append`, etc.). This is a known API design issue in
html5ever. The current implementation uses `#[expect(invalid_reference_casting)]`
with unsafe pointer casts, following the same pattern used by other html5ever
consumers (kuchiki, markup5ever_rcdom). A future refactor could use `RefCell`
or `UnsafeCell` for interior mutability.

---

## Nix Integration

- **Flake pattern**: substrate `rust-library.nix`
- **Outputs**: `packages`, `devShells`, `apps` (check-all, bump, publish, release, regenerate), `overlays.default`
- **System**: aarch64-darwin (macOS Apple Silicon)
- **Cargo.nix**: Generated via crate2nix, committed to repo

```nix
# Consumer flake.nix
nami-core = {
  url = "github:pleme-io/nami-core";
  inputs.nixpkgs.follows = "nixpkgs";
  inputs.substrate.follows = "substrate";
};
```

---

## Roadmap

### Phase 1 -- Core Pipeline [DONE]
DOM parsing (html5ever TreeSink), CSS parsing (lightningcss), basic cascade
resolution, taffy layout computation, content blocking with EasyList-format
filter lists, bookmarks and history storage, browser configuration.

### Phase 2 -- Consumer Migration [NEXT]
Replace duplicated dom/css/layout modules in aranami with nami-core imports.
Wire namimado to use nami-core for bookmarks, history, and content blocking.

### Phase 3 -- Cascade Correctness
Full CSS selector matching (class, ID, attribute, combinators). Specificity
calculation. Property inheritance. Shorthand expansion. Default UA stylesheet.

### Phase 4 -- Layout Completeness
Text measurement integration. Inline flow layout. Percentage units. Auto
margins. Table layout. Positioned elements. Float layout.

### Phase 5 -- Content Pipeline
`<style>` tag extraction from DOM. `<link rel="stylesheet">` fetching (via
todoku behind `network` feature). Inline `style=""` attribute parsing.

### Phase 6 -- Programmability
Content transform API for soushi/Rhai plugins. Reader mode transform.
DOM mutation API (insert, remove, modify elements). CSS injection.

### Phase 7 -- Performance
Arena-based DOM allocation. Incremental style resolution (dirty flags).
Layout caching. Selector index for fast matching. Parallel cascade.

### Phase 8 -- JavaScript
Optional boa_engine integration behind a feature flag. DOM bindings.
Event dispatch. setTimeout/setInterval. XMLHttpRequest/fetch polyfill.
