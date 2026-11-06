//! Can SQLite sessions record tables without a primary key, keyed by rowid,
//! and does a rewind then replay to the same rows as a replica that never
//! rewound?
//!
//! rusqlite 0.39 does not expose `sqlite3session_object_config`, and its
//! `Session` hides the raw handle, so `record` creates the session through
//! the FFI and hands the changeset back as bytes, which rusqlite's stream
//! functions invert and apply.

use std::ffi::{c_int, c_void};
use std::ptr;

use rusqlite::session::{ConflictAction, invert_strm};
use rusqlite::{Connection, ffi};

// Record the changes `work` makes as a changeset. With `by_rowid`, tables
// without a primary key are recorded too, keyed by their rowid.
pub fn record(connection: &Connection, by_rowid: bool, work: impl FnOnce(&Connection)) -> Vec<u8> {
  unsafe {
    let mut session = ptr::null_mut();
    assert_eq!(ffi::sqlite3session_create(connection.handle(), c"main".as_ptr(), &mut session), ffi::SQLITE_OK);
    let mut flag: c_int = by_rowid as c_int;
    let configured = ffi::sqlite3session_object_config(session, ffi::SQLITE_SESSION_OBJCONFIG_ROWID, &mut flag as *mut c_int as *mut c_void);
    assert_eq!((configured, flag), (ffi::SQLITE_OK, by_rowid as c_int));
    assert_eq!(ffi::sqlite3session_attach(session, ptr::null()), ffi::SQLITE_OK);
    work(connection);
    let (mut size, mut buffer): (c_int, *mut c_void) = (0, ptr::null_mut());
    assert_eq!(ffi::sqlite3session_changeset(session, &mut size, &mut buffer), ffi::SQLITE_OK);
    let bytes = if buffer.is_null() { vec![] } else { std::slice::from_raw_parts(buffer as *const u8, size as usize).to_vec() };
    ffi::sqlite3_free(buffer);
    ffi::sqlite3session_delete(session);
    bytes
  }
}

pub fn invert(changeset: &[u8]) -> Vec<u8> {
  let mut inverted = vec![];
  invert_strm(&mut &changeset[..], &mut inverted).unwrap();
  inverted
}

pub fn apply(connection: &Connection, changeset: &[u8]) {
  connection.apply_strm(&mut &changeset[..], None::<fn(&str) -> bool>, |_, _| ConflictAction::SQLITE_CHANGESET_ABORT).unwrap();
}

#[cfg(test)]
mod tests {
  use super::*;

  // A table of notes, declared one way or another.
  fn database(table: &str) -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    connection.execute_batch(table).unwrap();
    connection
  }

  fn insert(text: &'static str) -> impl FnOnce(&Connection) {
    move |connection| {
      connection.execute("INSERT INTO note (text) VALUES (?)", [text]).unwrap();
    }
  }

  fn rows(connection: &Connection) -> Vec<(i64, String)> {
    let mut statement = connection.prepare("SELECT rowid, text FROM note ORDER BY rowid").unwrap();
    statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?))).unwrap().map(Result::unwrap).collect()
  }

  const KEYLESS: &str = "CREATE TABLE note (text TEXT)";
  const ROWID_KEY: &str = "CREATE TABLE note (id INTEGER PRIMARY KEY, text TEXT)";
  const AUTOINCREMENT: &str = "CREATE TABLE note (id INTEGER PRIMARY KEY AUTOINCREMENT, text TEXT)";

  #[test]
  fn keyless_tables_are_ignored_by_default() {
    let connection = database(KEYLESS);
    assert!(record(&connection, false, insert("a")).is_empty());
  }

  #[test]
  fn the_rowid_option_records_keyless_tables_and_the_inverse_undoes_them() {
    let connection = database(KEYLESS);
    insert("a")(&connection);
    let changeset = record(&connection, true, |connection| {
      connection.execute("UPDATE note SET text = 'edited'", []).unwrap();
      insert("b")(connection);
    });
    assert!(!changeset.is_empty());
    apply(&connection, &invert(&changeset));
    assert_eq!(rows(&connection), vec![(1, "a".to_string())]);
  }

  // Replica X applies a then b, then op c arrives late but sorts before b, so
  // X rewinds b, applies c and re-runs b. Replica Y receives all three in
  // order and never rewinds. Both should end with the same rows.
  fn rewound_and_straight(table: &str) -> (Vec<(i64, String)>, Vec<(i64, String)>) {
    let rewound = database(table);
    record(&rewound, true, insert("a"));
    let b = record(&rewound, true, insert("b"));
    apply(&rewound, &invert(&b));
    record(&rewound, true, insert("c"));
    record(&rewound, true, insert("b"));

    let straight = database(table);
    for text in ["a", "c", "b"] {
      insert(text)(&straight);
    }
    (rows(&rewound), rows(&straight))
  }

  #[test]
  fn keyless_and_rowid_keyed_tables_replay_to_the_same_rows_after_a_rewind() {
    for table in [KEYLESS, ROWID_KEY] {
      let (rewound, straight) = rewound_and_straight(table);
      assert_eq!(rewound, straight, "{table}");
    }
  }

  // A changeset kept for a later rewind names rows by rowid, so it only stays
  // valid while those rowids do.
  #[test]
  fn vacuum_renumbers_a_keyless_table_but_not_an_integer_key() {
    let renumbered = |table: &str| {
      let path = std::env::temp_dir().join(format!("session-rowid-{}-{}", std::process::id(), table.len()));
      let _ = std::fs::remove_file(&path);
      let connection = Connection::open(&path).unwrap();
      connection.execute_batch(table).unwrap();
      for text in ["a", "b", "c"] {
        insert(text)(&connection);
      }
      connection.execute("DELETE FROM note WHERE text = 'b'", []).unwrap();
      connection.execute_batch("VACUUM").unwrap();
      let after = rows(&connection);
      std::fs::remove_file(&path).unwrap();
      after
    };
    assert_eq!(renumbered(KEYLESS), vec![(1, "a".to_string()), (2, "c".to_string())]);
    assert_eq!(renumbered(ROWID_KEY), vec![(1, "a".to_string()), (3, "c".to_string())]);
  }

  // The id SQLite assigns depends on the rows already there, so a late op that
  // sorts first moves it: b was 2 when its author saw it, and is 3 after the
  // rewind. A later op that names note 2 now edits c instead.
  #[test]
  fn an_assigned_rowid_moves_under_its_author_after_a_rewind() {
    for table in [KEYLESS, ROWID_KEY] {
      let replica = database(table);
      record(&replica, true, insert("a"));
      let b = record(&replica, true, insert("b"));
      let seen_by_author = replica.last_insert_rowid();
      apply(&replica, &invert(&b));
      record(&replica, true, insert("c"));
      record(&replica, true, insert("b"));
      let text: String = replica.query_row("SELECT text FROM note WHERE rowid = ?", [seen_by_author], |row| row.get(0)).unwrap();
      assert_eq!((seen_by_author, text.as_str()), (2, "c"), "{table}");
    }
  }

  // A WITHOUT ROWID table always has a primary key, never assigns one, and
  // refuses AUTOINCREMENT, so its keys can only come from the op.
  const WITHOUT_ROWID: &str = "CREATE TABLE note (id TEXT PRIMARY KEY, text TEXT) WITHOUT ROWID";

  #[test]
  fn a_without_rowid_table_is_recorded_and_refuses_a_missing_key() {
    let connection = database(WITHOUT_ROWID);
    let changeset = record(&connection, false, |connection| {
      connection.execute("INSERT INTO note (id, text) VALUES ('op-1', 'a')", []).unwrap();
    });
    apply(&connection, &invert(&changeset));
    assert_eq!(connection.query_row("SELECT count(*) FROM note", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    assert!(connection.execute("INSERT INTO note (text) VALUES ('b')", []).is_err());
    assert!(Connection::open_in_memory().unwrap().execute_batch("CREATE TABLE t (id INTEGER PRIMARY KEY AUTOINCREMENT) WITHOUT ROWID").is_err());
  }

  // Can the schema itself fill an omitted key from the op? `op_id()` stands in
  // for the op's draws: a SQL function the core would register per op.
  fn with_op_ids(table: &str) -> rusqlite::Result<Connection> {
    use rusqlite::functions::FunctionFlags;
    let connection = Connection::open_in_memory()?;
    let drawn = std::cell::Cell::new(100);
    connection.create_scalar_function("op_id", 0, FunctionFlags::SQLITE_UTF8, move |_| {
      drawn.set(drawn.get() + 1);
      Ok(drawn.get())
    })?;
    connection.execute_batch(table)?;
    Ok(connection)
  }

  #[test]
  fn a_default_can_draw_an_omitted_key_from_the_op() {
    let without_rowid = with_op_ids("CREATE TABLE note (id INTEGER PRIMARY KEY DEFAULT (op_id()), text TEXT) WITHOUT ROWID").unwrap();
    insert("a")(&without_rowid);
    without_rowid.execute_batch("INSERT INTO note (text) VALUES ('b'), ('c'); INSERT INTO note (text) SELECT text || '!' FROM note").unwrap();
    let mut statement = without_rowid.prepare("SELECT id, text FROM note ORDER BY id").unwrap();
    let rows: Vec<(i64, String)> = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?))).unwrap().map(Result::unwrap).collect();
    let expected = [(101, "a"), (102, "b"), (103, "c"), (104, "a!"), (105, "b!"), (106, "c!")];
    assert_eq!(rows, expected.map(|(id, text)| (id, text.to_string())));
  }

  // In an ordinary table an INTEGER PRIMARY KEY is the rowid, and SQLite
  // assigns it before any default applies.
  #[test]
  fn a_rowid_key_ignores_its_default() {
    let rowid = with_op_ids("CREATE TABLE note (id INTEGER PRIMARY KEY DEFAULT (op_id()), text TEXT)").unwrap();
    insert("a")(&rowid);
    let id: i64 = rowid.query_row("SELECT id FROM note", [], |row| row.get(0)).unwrap();
    assert_eq!(id, 1);
  }

  // The tables a changeset changes, with the count of rows changed in each:
  // what the core would take as an op's write set.
  fn changed(changeset: &[u8]) -> Vec<(String, usize)> {
    use rusqlite::fallible_streaming_iterator::FallibleStreamingIterator;
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    let input: &mut dyn std::io::Read = &mut &changeset[..];
    let mut items = rusqlite::session::ChangesetIter::start_strm(&input).unwrap();
    while let Some(item) = items.next().unwrap() {
      *counts.entry(item.op().unwrap().table_name().to_string()).or_default() += 1;
    }
    counts.into_iter().collect()
  }

  const NOTES_AND_TAGS: &str = "CREATE TABLE note (id TEXT PRIMARY KEY, text TEXT) WITHOUT ROWID;
    CREATE TABLE tag (id TEXT PRIMARY KEY) WITHOUT ROWID";

  // A call nested in an op runs under a savepoint, and rolls back alone when
  // it fails. The op's changeset must then hold only what the op kept.
  #[test]
  fn a_savepoint_rolled_back_inside_the_transaction_leaves_nothing_in_the_changeset() {
    let connection = database(NOTES_AND_TAGS);
    let changeset = record(&connection, false, |connection| {
      connection
        .execute_batch(
          "BEGIN;
           INSERT INTO note VALUES ('n1', 'kept');
           SAVEPOINT nested;
           INSERT INTO tag VALUES ('t1');
           UPDATE note SET text = 'discarded' WHERE id = 'n1';
           ROLLBACK TO nested;
           RELEASE nested;
           COMMIT;",
        )
        .unwrap();
    });
    assert_eq!(changed(&changeset), vec![("note".to_string(), 1)]);
    apply(&connection, &invert(&changeset));
    assert_eq!(connection.query_row("SELECT count(*) FROM note", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
  }

  // The authorizer counts a table as written when a statement naming it is
  // prepared; a changeset counts only rows whose values changed.
  #[test]
  fn a_changeset_holds_only_net_changes() {
    let connection = database(NOTES_AND_TAGS);
    connection.execute("INSERT INTO note VALUES ('n1', 'same')", []).unwrap();
    let changeset = record(&connection, false, |connection| {
      connection
        .execute_batch(
          "UPDATE tag SET id = 'x' WHERE id = 'none';
           UPDATE note SET text = 'same' WHERE id = 'n1';
           INSERT INTO tag VALUES ('t1');
           DELETE FROM tag WHERE id = 't1';",
        )
        .unwrap();
    });
    assert_eq!(changed(&changeset), vec![]);
  }

  #[test]
  fn autoincrement_keys_drift_after_a_rewind() {
    let (rewound, straight) = rewound_and_straight(AUTOINCREMENT);
    assert_eq!(straight, vec![(1, "a".to_string()), (2, "c".to_string()), (3, "b".to_string())]);
    assert_eq!(rewound, vec![(1, "a".to_string()), (3, "c".to_string()), (4, "b".to_string())]);
  }
}
