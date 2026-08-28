use rusqlite::Connection;
use std::path::Path;

// ponytail: ~30s one-time C compile for bundled SQLite; cached by Cargo build cache.
// Second SQLite copy in graph (server uses sea-orm driver) but isolated to this crate.

pub fn open_ro(path: &Path) -> rusqlite::Result<Connection> {
    Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .and_then(|conn| {
        conn.busy_timeout(std::time::Duration::from_millis(1000))?;
        Ok(conn)
    })
}

/// Open read-only + `immutable=1` via a SQLite URI. For databases owned by a
/// live application (e.g. an IDE's state DB): immutable mode never takes any
/// lock, so we cannot stall or corrupt the owner's writes — at the cost of
/// possibly reading a slightly stale snapshot mid-write.
pub fn open_ro_immutable(path: &Path) -> rusqlite::Result<Connection> {
    // Percent-encode the URI metacharacters so an unusual path can't be
    // misparsed as query parameters (SQLite percent-decodes the URI path).
    let escaped = path
        .to_string_lossy()
        .replace('%', "%25")
        .replace('?', "%3F")
        .replace('#', "%23");
    let uri = format!("file:{escaped}?immutable=1");
    Connection::open_with_flags(
        uri,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
            | rusqlite::OpenFlags::SQLITE_OPEN_URI
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
}
