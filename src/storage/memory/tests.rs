use super::MemoryStore;
use std::sync::OnceLock;

/// Register the sqlite-vec extension exactly once per test process.
/// `MemoryStore::migrate()` creates a `vec0` virtual table, which
/// requires the extension to be loaded before any connection is opened.
fn register_sqlite_vec() {
    static INIT: OnceLock<()> = OnceLock::new();
    INIT.get_or_init(|| {
        #[allow(clippy::missing_transmute_annotations)]
        unsafe {
            rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
                sqlite_vec::sqlite3_vec_init as *const (),
            )));
        }
    });
}

fn open_store() -> MemoryStore {
    register_sqlite_vec();
    MemoryStore::open(std::path::Path::new(":memory:"))
        .expect("failed to open in-memory MemoryStore")
}

fn count_edges(store: &MemoryStore, from_id: i64, to_id: i64, kind: &str) -> i64 {
    store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM memory_edges WHERE from_id = ?1 AND to_id = ?2 AND kind = ?3",
            rusqlite::params![from_id, to_id, kind],
            |r| r.get(0),
        )
        .unwrap_or(0)
}

// ── supersede() ──────────────────────────────────────────────────────────────

#[test]
fn supersede_happy_path() {
    let store = open_store();

    let old_id = store
        .add_note("decision", "Old decision", "old body", &[], &[], None, None)
        .unwrap();
    let new_id = store
        .add_note("decision", "New decision", "new body", &[], &[], None, None)
        .unwrap();

    let changed = store.supersede(old_id, new_id).unwrap();
    assert!(changed, "supersede() should return true on first call");

    // (a) old note must be archived with superseded_by set
    let old_note = store.get(old_id).unwrap().expect("old note must exist");
    assert_eq!(old_note.status, "archived");
    assert_eq!(old_note.superseded_by, Some(new_id));

    // (b) a memory_edges row must exist linking new → old
    assert_eq!(
        count_edges(&store, new_id, old_id, "supersedes"),
        1,
        "expected exactly one supersedes edge"
    );
}

#[test]
fn supersede_idempotent() {
    let store = open_store();

    let old_id = store
        .add_note("note", "Alpha", "body", &[], &[], None, None)
        .unwrap();
    let new_id = store
        .add_note("note", "Beta", "body", &[], &[], None, None)
        .unwrap();

    let first = store.supersede(old_id, new_id).unwrap();
    assert!(first);

    // Second call on an already-archived note must return false
    let second = store.supersede(old_id, new_id).unwrap();
    assert!(
        !second,
        "supersede() should return false when note is already archived"
    );

    // Must not have inserted a duplicate edge
    assert_eq!(
        count_edges(&store, new_id, old_id, "supersedes"),
        1,
        "duplicate supersedes edge must not be inserted"
    );
}

// ── add_edge() ───────────────────────────────────────────────────────────────

#[test]
fn add_edge_valid_kinds_accepted() {
    let store = open_store();
    let a = store
        .add_note("note", "A", "", &[], &[], None, None)
        .unwrap();
    let b = store
        .add_note("note", "B", "", &[], &[], None, None)
        .unwrap();

    for kind in ["supersedes", "relates_to", "contradicts"] {
        store
            .add_edge(a, b, kind)
            .unwrap_or_else(|e| panic!("add_edge with kind '{kind}' failed: {e}"));
    }
}

#[test]
fn add_edge_invalid_kind_returns_err() {
    let store = open_store();
    let a = store
        .add_note("note", "A", "", &[], &[], None, None)
        .unwrap();
    let b = store
        .add_note("note", "B", "", &[], &[], None, None)
        .unwrap();

    let err = store
        .add_edge(a, b, "invented")
        .expect_err("add_edge with invalid kind must return Err");
    assert!(
        err.to_string().contains("invented"),
        "error message must mention the invalid kind; got: {err}"
    );
}

#[test]
fn add_edge_duplicate_silently_ignored() {
    let store = open_store();
    let a = store
        .add_note("note", "A", "", &[], &[], None, None)
        .unwrap();
    let b = store
        .add_note("note", "B", "", &[], &[], None, None)
        .unwrap();

    store.add_edge(a, b, "relates_to").unwrap();
    store.add_edge(a, b, "relates_to").unwrap(); // second call must not error

    assert_eq!(
        count_edges(&store, a, b, "relates_to"),
        1,
        "duplicate edge must not produce a second row"
    );
}
