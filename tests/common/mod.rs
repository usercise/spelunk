//! Shared test helpers.
#![allow(dead_code)]
//!
//! Import with `mod common;` or `use crate::common::*;` inside integration tests.

use std::sync::OnceLock;

/// Register the sqlite-vec extension exactly once for the test process.
///
/// sqlite3_auto_extension is process-global; calling it more than once per
/// address is a no-op but calling it from multiple threads without
/// synchronisation is UB.  `OnceLock` guarantees single initialisation.
///
/// Tests that open a `Database` or `ServerDb` **must** call this first.
/// Annotate those tests with `#[serial_test::serial]` so the global
/// registration happens before any connection is opened.
pub fn register_sqlite_vec() {
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

/// Open an in-memory `spelunk::storage::Database` for tests.
///
/// Calls `register_sqlite_vec()` automatically.
pub fn open_test_db() -> spelunk::storage::Database {
    register_sqlite_vec();
    spelunk::storage::Database::open(std::path::Path::new(":memory:"))
        .expect("failed to open in-memory database")
}

/// Open an in-memory `spelunk::server::db::ServerDb` for tests.
///
/// Calls `register_sqlite_vec()` automatically.
pub fn open_test_server_db(dim: usize) -> spelunk::server::db::ServerDb {
    register_sqlite_vec();
    spelunk::server::db::ServerDb::open(std::path::Path::new(":memory:"), dim)
        .expect("failed to open in-memory server database")
}
