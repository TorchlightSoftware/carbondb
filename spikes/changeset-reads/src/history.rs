//! A store kept as the core keeps one: each op runs in its own transaction
//! and records its place, its changeset, the commit it started from, and the
//! rows it wrote. The WAL is never checkpointed, so a row read as of a place
//! can be read both ways on one file: at the snapshot the core opens now
//! (`read_at_snapshot`), or rebuilt from later changesets (`read_rebuilt`).

use std::path::{Path, PathBuf};

use fallible_streaming_iterator::FallibleStreamingIterator;
use rusqlite::session::{ChangesetIter, Session};
use rusqlite::types::Value;
use rusqlite::{Connection, OpenFlags, OptionalExtension, Result, ffi, params};

use crate::{Columns, Rows, Table, change_to, named, query, row_as_of};

// The table the measurements read: one row per member, keyed by id, with a
// collation and an integer column, as a service's table would have.
pub const MEMBER: &str = "CREATE TABLE member (id TEXT PRIMARY KEY, name TEXT COLLATE NOCASE, role TEXT, joined INTEGER) WITHOUT ROWID";

// The core's record of each op, as `carbon_pointer`, `carbon_result` and
// `carbon_write` keep it, except that `carbon_write` keeps each writer's place
// beside the row, so the writers after a place are one range of the index.
const CORE: &str = "
  CREATE TABLE carbon_pointer (hash TEXT PRIMARY KEY, time INTEGER NOT NULL);
  CREATE TABLE carbon_result (hash TEXT PRIMARY KEY, changeset BLOB NOT NULL, snapshot BLOB NOT NULL);
  CREATE TABLE carbon_write (tbl TEXT, key TEXT, time INTEGER, hash TEXT, PRIMARY KEY (tbl, key, time, hash)) WITHOUT ROWID;
  CREATE INDEX carbon_write_by_op ON carbon_write (hash);";

// The ops placed after a time that wrote a row.
const WRITERS: &str = "FROM carbon_write w JOIN carbon_result r USING (hash) WHERE w.tbl = ?1 AND w.key = ?2 AND w.time > ?3";

pub struct Store {
  pub path: PathBuf,
  writer: Connection,
}

impl Store {
  // A store holding one service table, kept in WAL mode and never checkpointed.
  pub fn create(path: &Path, table: &str) -> Result<Store> {
    let writer = Connection::open(path)?;
    writer.pragma_update(None, "journal_mode", "wal")?;
    writer.pragma_update(None, "wal_autocheckpoint", 0)?;
    writer.execute_batch(&format!("{table}; {CORE}"))?;
    Ok(Store { path: path.into(), writer })
  }

  // A read-only connection, as a GET's reader opens.
  pub fn reader(&self) -> Result<Connection> {
    Connection::open_with_flags(&self.path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)
  }

  // Apply an op placed at a time, as dispatch does: note the commit it
  // starts from, run it in one transaction recording its changeset, and
  // record its place, result and the rows it wrote in the same transaction.
  pub fn apply(&self, time: i64, sql: &str) -> Result<()> {
    self.writer.execute_batch("BEGIN; SELECT count(*) FROM sqlite_schema;")?;
    let start = snapshot_of(&self.writer)?;
    self.writer.execute_batch("COMMIT; BEGIN IMMEDIATE")?;
    let mut session = Session::new(&self.writer)?;
    session.attach(Some("member"))?;
    self.writer.execute_batch(sql)?;
    let mut changeset = vec![];
    session.changeset_strm(&mut changeset)?;
    drop(session);
    let hash = format!("op-{time}");
    self.writer.execute("INSERT INTO carbon_pointer VALUES (?, ?)", params![hash, time])?;
    self.writer.execute("INSERT INTO carbon_result VALUES (?, ?, ?)", params![hash, changeset, start.to_vec()])?;
    for (table, key) in rows(&changeset)? {
      self.writer.execute("INSERT INTO carbon_write VALUES (?, ?, ?, ?)", params![table, key_of(&key), time, hash])?;
    }
    self.writer.execute_batch("COMMIT")
  }

  // Bytes the WAL holds, and the changesets and snapshots the results hold.
  pub fn sizes(&self) -> Result<(u64, i64, i64)> {
    let wal = std::fs::metadata(self.path.with_extension("db-wal")).map_err(|error| rusqlite::Error::ToSqlConversionFailure(error.into()))?.len();
    let (changesets, snapshots) = self.writer.query_row("SELECT sum(length(changeset)), sum(length(snapshot)) FROM carbon_result", [], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok((wal, changesets, snapshots))
  }

  // Read a row of `member` as of a place as the core does now: at the commit
  // the first op placed after it that wrote the row started from, or as it is
  // now when none did.
  pub fn read_at_snapshot(&self, reader: &Connection, key: &[Value], time: i64, sql: &str, params: &[Value]) -> Result<Rows> {
    let first: Option<Vec<u8>> = reader.query_row(&format!("SELECT r.snapshot {WRITERS} ORDER BY w.time LIMIT 1"), params!["member", key_of(key), time], |row| row.get(0)).optional()?;
    reader.execute_batch("BEGIN; SELECT count(*) FROM sqlite_schema;")?;
    let found = first.map_or(Ok(()), |bytes| open_at(reader, &bytes)).and_then(|_| query(reader, sql, params));
    reader.execute_batch("COMMIT")?;
    found
  }

  // Read a row of a table as of a place rebuilt from changesets: the row as
  // it is now, with each later op's change to it undone, newest first; then
  // run the statement against it with `run`.
  pub fn read_rebuilt(&self, reader: &Connection, table: &Table, key: &[Value], time: i64, run: impl FnOnce(Option<&Columns>) -> Result<Rows>) -> Result<Rows> {
    reader.execute_batch("BEGIN")?;
    let found = rebuild(reader, table, key, time).and_then(|row| run(row.as_ref()));
    reader.execute_batch("COMMIT")?;
    found
  }
}

fn rebuild(reader: &Connection, table: &Table, key: &[Value], time: i64) -> Result<Option<Columns>> {
  let now = table.row(reader, key)?;
  let mut later = reader.prepare_cached(&format!("SELECT r.changeset {WRITERS} ORDER BY w.time DESC"))?;
  let changesets: Vec<Vec<u8>> = later.query_map(params![table.name, key_of(key), time], |row| row.get(0))?.collect::<Result<_>>()?;
  let changes: Vec<_> = changesets.iter().map(|changeset| change_to(changeset, &table.name, key)).collect::<Result<Vec<_>>>()?;
  Ok(row_as_of(now, changes.into_iter().flatten()))
}

// A row's key as `carbon_write` keeps it.
fn key_of(key: &[Value]) -> String {
  format!("{key:?}")
}

// The rows a changeset changes, by table and key.
fn rows(changeset: &[u8]) -> Result<Vec<(String, Columns)>> {
  let input: &mut dyn std::io::Read = &mut &changeset[..];
  let mut items = ChangesetIter::start_strm(&input)?;
  let mut found = vec![];
  while let Some(item) = items.next()? {
    found.push((item.op()?.table_name().to_string(), named(item)?));
  }
  Ok(found)
}

// The commit a connection's open read sees, as SQLite names it.
fn snapshot_of(connection: &Connection) -> Result<[u8; 48]> {
  let mut taken: *mut ffi::sqlite3_snapshot = std::ptr::null_mut();
  // SAFETY: the handle is the connection's own; the snapshot is copied, then freed.
  unsafe {
    checked(ffi::sqlite3_snapshot_get(connection.handle(), c"main".as_ptr(), &mut taken))?;
    let snapshot = (*taken).hidden;
    ffi::sqlite3_snapshot_free(taken);
    Ok(snapshot)
  }
}

// Move a connection's open read, which has read something, to an earlier commit.
fn open_at(connection: &Connection, snapshot: &[u8]) -> Result<()> {
  let mut opened = ffi::sqlite3_snapshot { hidden: snapshot.try_into().expect("a snapshot is 48 bytes") };
  // SAFETY: the handle is the connection's own, and SQLite only reads the snapshot.
  checked(unsafe { ffi::sqlite3_snapshot_open(connection.handle(), c"main".as_ptr(), &mut opened) })
}

fn checked(code: std::ffi::c_int) -> Result<()> {
  match code {
    ffi::SQLITE_OK => Ok(()),
    code => Err(rusqlite::Error::SqliteFailure(ffi::Error::new(code), None)),
  }
}
