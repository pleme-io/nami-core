//! Snapshot, shadow, and put-back operations for the DOM.
//!
//! The workflow the browser wants:
//!
//!   parse → [`Snapshot`] — immutable, content-addressable (BLAKE3)
//!                          handle to the full DOM. Cheap to clone
//!                          (`Arc`), cheap to store.
//!     ↓
//!   [`Snapshot::shadow`] — materialise a mutable working copy
//!     ↓
//!   user/agent mutates the [`Shadow`] — apply transforms, splice,
//!     rewrite, whatever
//!     ↓
//!   [`Shadow::freeze`] — turn the mutated working copy into a NEW
//!     Snapshot with a fresh hash
//!     ↓
//!   [`Shadow::put_back`] — overwrite the original owning location
//!     (the browser's tab, a file, an MCP response)
//!
//! Snapshots are structural — the whole `Document` lives behind an
//! `Arc<Document>` so cloning the snapshot handle is O(1). The hash
//! is computed over the deterministic S-expression serialization
//! (`lisp::dom_to_sexp_with` with compact + trim settings) so two
//! documents with byte-identical trees always produce the same root
//! hash regardless of how they were constructed.

use crate::dom::Document;
use crate::lisp::{SexpOptions, dom_to_sexp_with, sexp_to_dom};
use std::sync::Arc;

/// Canonical S-expression serialization used for hashing + diffing.
/// Same settings, always — so the hash is a pure function of the tree.
fn canon_opts() -> SexpOptions {
    SexpOptions {
        depth_cap: None,
        pretty: false,
        trim_whitespace: true,
    }
}

/// Immutable, content-addressable handle to a DOM.
///
/// Cheap to clone (`Arc<Document>` + 32-byte hash). Multiple parts of
/// the system can hold the same `Snapshot` without copying the tree.
#[derive(Debug, Clone)]
pub struct Snapshot {
    doc: Arc<Document>,
    hash: [u8; 32],
    /// Cached canonical serialization — deduplicates work when
    /// multiple consumers want to serialize or hash the same snap.
    canon: Arc<String>,
}

impl Snapshot {
    /// Take a snapshot of a document.
    #[must_use]
    pub fn of(doc: Document) -> Self {
        let doc = Arc::new(doc);
        let canon = dom_to_sexp_with(&doc, &canon_opts());
        let hash: [u8; 32] = blake3::hash(canon.as_bytes()).into();
        Self {
            doc,
            hash,
            canon: Arc::new(canon),
        }
    }

    /// Parse HTML directly into a snapshot.
    #[must_use]
    pub fn from_html(html: &str) -> Self {
        Self::of(Document::parse(html))
    }

    /// The underlying document. `Arc::clone` for cheap sharing.
    #[must_use]
    pub fn document(&self) -> &Document {
        &self.doc
    }

    /// 32-byte BLAKE3 hash over the canonical serialization.
    #[must_use]
    pub fn hash(&self) -> &[u8; 32] {
        &self.hash
    }

    /// Hex-encoded hash, handy for logs + attestation contexts.
    #[must_use]
    pub fn hex(&self) -> String {
        let mut out = String::with_capacity(64);
        for b in &self.hash {
            use std::fmt::Write;
            let _ = write!(out, "{b:02x}");
        }
        out
    }

    /// Canonical S-expression form (shared `Arc<String>`).
    #[must_use]
    pub fn sexp(&self) -> Arc<String> {
        Arc::clone(&self.canon)
    }

    /// Allocate a mutable working copy. The shadow starts with a
    /// back-reference to this snapshot so diffing against the
    /// original is a one-liner after mutation.
    #[must_use]
    pub fn shadow(&self) -> Shadow {
        Shadow {
            original: self.clone(),
            current: (*self.doc).clone(),
        }
    }
}

/// A mutable working copy of a [`Snapshot`].
///
/// Hold onto `original` so the before/after comparison is trivial:
/// `shadow.changed()` compares hashes; `shadow.diff_sexp()` shows
/// the canonical diff.
#[derive(Debug)]
pub struct Shadow {
    original: Snapshot,
    current: Document,
}

impl Shadow {
    /// Mutable access to the working document.
    #[must_use]
    pub fn document_mut(&mut self) -> &mut Document {
        &mut self.current
    }

    /// Immutable access to the working document.
    #[must_use]
    pub fn document(&self) -> &Document {
        &self.current
    }

    /// Snapshot the original we started from (cheap clone).
    #[must_use]
    pub fn original(&self) -> &Snapshot {
        &self.original
    }

    /// Freeze the current state into a new snapshot. Does not mutate
    /// `self` — you can continue editing after freezing.
    #[must_use]
    pub fn freeze(&self) -> Snapshot {
        Snapshot::of(self.current.clone())
    }

    /// True if the working copy differs from the original.
    /// Cheap — compares hashes of the canonical forms.
    #[must_use]
    pub fn changed(&self) -> bool {
        self.freeze().hash() != self.original.hash()
    }

    /// Consume the shadow and return the mutated document, paired
    /// with a fresh snapshot. The typical "put back" shape.
    #[must_use]
    pub fn commit(self) -> (Document, Snapshot) {
        let frozen = Snapshot::of(self.current.clone());
        (self.current, frozen)
    }

    /// Replace the working document wholesale — e.g. after parsing
    /// an externally-rewritten S-expression back in.
    pub fn put_back(&mut self, doc: Document) {
        self.current = doc;
    }

    /// Convenience: replace the working document from an
    /// S-expression string. Used when an agent emitted a mutated
    /// Lisp tree and we're splicing it back into the live DOM.
    pub fn put_back_sexp(&mut self, sexp: &str) -> Result<(), String> {
        let doc = sexp_to_dom(sexp)?;
        self.current = doc;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_hash_is_deterministic() {
        let s1 = Snapshot::from_html("<html><body><p>hi</p></body></html>");
        let s2 = Snapshot::from_html("<html><body><p>hi</p></body></html>");
        assert_eq!(s1.hash(), s2.hash());
        assert_eq!(s1.hex(), s2.hex());
    }

    #[test]
    fn snapshot_hash_changes_with_content() {
        let a = Snapshot::from_html("<html><body><p>hi</p></body></html>");
        let b = Snapshot::from_html("<html><body><p>bye</p></body></html>");
        assert_ne!(a.hash(), b.hash());
    }

    #[test]
    fn shadow_roundtrip_preserves_unchanged_hash() {
        // Take a snapshot, make a shadow, don't mutate → freeze should
        // yield the same hash.
        let snap = Snapshot::from_html("<html><body><p>hi</p></body></html>");
        let shadow = snap.shadow();
        assert!(!shadow.changed());
        let frozen = shadow.freeze();
        assert_eq!(snap.hash(), frozen.hash());
    }

    #[test]
    fn shadow_detects_mutation() {
        let snap = Snapshot::from_html("<html><body><p>hi</p></body></html>");
        let mut shadow = snap.shadow();
        // Mutate by replacing the document from a new parse.
        shadow.put_back(Document::parse("<html><body><p>bye</p></body></html>"));
        assert!(shadow.changed());
        let frozen = shadow.freeze();
        assert_ne!(snap.hash(), frozen.hash());
    }

    #[test]
    fn shadow_put_back_sexp_mutates() {
        let snap = Snapshot::from_html("<html><body><p>hi</p></body></html>");
        let mut shadow = snap.shadow();
        // Canonical sexp of a different document:
        let alt = r#"(document (element :tag "html" (element :tag "body" (element :tag "p" (text "totally different")))))"#;
        shadow.put_back_sexp(alt).unwrap();
        assert!(shadow.changed());
        assert!(
            shadow
                .document()
                .text_content()
                .contains("totally different")
        );
    }

    #[test]
    fn commit_yields_document_plus_fresh_snapshot() {
        let snap = Snapshot::from_html("<html><body><p>one</p></body></html>");
        let mut shadow = snap.shadow();
        shadow.put_back(Document::parse("<html><body><p>two</p></body></html>"));
        let (doc, fresh) = shadow.commit();
        assert!(doc.text_content().contains("two"));
        assert_ne!(fresh.hash(), snap.hash());
        // fresh.hash should match re-snapshot of the committed doc
        assert_eq!(fresh.hash(), Snapshot::of(doc).hash());
    }

    #[test]
    fn canonical_sexp_is_sharable() {
        let snap = Snapshot::from_html("<html><body><p>shared</p></body></html>");
        let s1 = snap.sexp();
        let s2 = snap.sexp();
        assert!(Arc::ptr_eq(&s1, &s2));
    }

    #[test]
    fn snapshot_clone_is_cheap_and_equal() {
        let a = Snapshot::from_html("<html><body><p>hi</p></body></html>");
        let b = a.clone();
        assert_eq!(a.hash(), b.hash());
        // Arc pointer equality — clone didn't deep-copy.
        assert!(Arc::ptr_eq(&a.sexp(), &b.sexp()));
    }

    #[test]
    fn put_back_sexp_rejects_garbage() {
        let snap = Snapshot::from_html("<html><body></body></html>");
        let mut shadow = snap.shadow();
        assert!(shadow.put_back_sexp("not a lisp doc").is_err());
    }
}
