use rusqlite::{Connection, OpenFlags};
use std::path::Path;

pub fn open(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("cannot create database directory: {error}"))?;
    }
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(|error| format!("cannot open database: {error}"))?;
    configure(&connection)?;
    migrate(&connection)?;
    Ok(connection)
}

pub fn open_in_memory() -> Result<Connection, String> {
    let connection =
        Connection::open_in_memory().map_err(|error| format!("cannot open database: {error}"))?;
    configure(&connection)?;
    migrate(&connection)?;
    Ok(connection)
}

fn configure(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA foreign_keys=ON;
             PRAGMA busy_timeout=5000;
             PRAGMA synchronous=FULL;",
        )
        .map_err(|error| format!("cannot configure database: {error}"))
}

pub fn migrate(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(include_str!("schema.sql"))
        .map_err(|error| format!("cannot migrate database: {error}"))
}

pub fn now_ns() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}
