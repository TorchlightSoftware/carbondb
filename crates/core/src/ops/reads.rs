//! Reads across services in a channel, through `callsync`: the ops that
//! changed a row after the reader's place, whose changes a read as of that
//! place undoes (`store::as_of`), and the record of who read it.
//!
//! Both indexes are keyed by table, key and place, so the writers or readers
//! of a row placed after a place are one range of the index, however many
//! there are before it.
//!
//! The op reading is the read service's reader. Its reads are recorded in the
//! read service's store, with its place and the store it applies in, so an op
//! applied or undone there at an earlier place, which changes what it read,
//! re-runs it (`readers_of`).

use serde_json::{Value, json};

use crate::Db;
use crate::error::Result;
use crate::ops::pointers::Pointer;
use crate::store::changeset::Row;

// The op whose reads a call makes, by its place in the shared order, and the
// store it applies in: an op, and every `callsync` read made inside it.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
  pub place: Pointer,
  pub store: String,
}

// Index rows of one table row placed after (?, ?, ?) in the shared order;
// both indexes keep a device of '' for none.
const AFTER: &str = "tbl = ? AND key = ? AND (time, device, %hash) > (?, ?, ?)";

fn after(row: &Row, place: &Pointer, hash: &str) -> (String, Vec<Value>) {
  let params = vec![json!(row.table), json!(row.key), json!(place.time), json!(place.device.as_deref().unwrap_or_default()), json!(place.hash)];
  (AFTER.replace("%hash", hash), params)
}

// The changesets of the ops placed after a place that changed a row, newest
// first: undone in turn on the row as it is now, they leave it as it stood then.
pub fn later_changesets(db: &dyn Db, row: &Row, place: &Pointer) -> Result<Vec<Vec<u8>>> {
  let (after, params) = after(row, place, "hash");
  let sql = format!("SELECT r.changeset FROM carbon_write w JOIN carbon_result r USING (hash) WHERE {after} ORDER BY time DESC, device DESC, hash DESC");
  // A changeset reads back through the JSON binding as an array of bytes.
  db.all(&sql, &params)?.into_iter().map(|found| Ok(serde_json::from_value(found["changeset"].clone())?)).collect()
}

// Record that a reader read these rows; a row it read before stays recorded once.
pub fn record(db: &dyn Db, rows: &[Row], reading: &Reading) -> Result<()> {
  let place = &reading.place;
  rows.iter().try_for_each(|row| {
    let params = [json!(row.table), json!(row.key), json!(place.time), json!(place.device.as_deref().unwrap_or_default()), json!(place.hash), json!(reading.store)];
    db.run("INSERT OR REPLACE INTO carbon_read (tbl, key, time, device, reader, store) VALUES (?, ?, ?, ?, ?, ?)", &params).map(|_| ())
  })
}

// The readers placed after an op that read any of the rows it changed, each
// once, in the shared order: what it changed is what they should have seen.
pub fn readers_of(db: &dyn Db, rows: &[Row], placed: &Pointer) -> Result<Vec<Reading>> {
  let found = rows.iter().map(|row| {
    let (after, params) = after(row, placed, "reader");
    db.all(&format!("SELECT reader AS hash, store, time, NULLIF(device, '') AS device FROM carbon_read WHERE {after}"), &params)
  });
  let mut readers: Vec<Reading> = found.collect::<Result<Vec<Vec<Value>>>>()?.into_iter().flatten().map(|row| Reading { place: Pointer::of(&row), store: row["store"].as_str().unwrap_or_default().to_string() }).collect();
  readers.sort_by(|one, other| (one.place.time, &one.place.device, &one.place.hash).cmp(&(other.place.time, &other.place.device, &other.place.hash)));
  readers.dedup();
  Ok(readers)
}

// Forget what readers read, as they are taken up again to read anew.
pub fn forget(db: &dyn Db, readers: &[Reading]) -> Result<()> {
  readers.iter().try_for_each(|reader| db.run("DELETE FROM carbon_read WHERE reader = ?", &[json!(reader.place.hash)]).map(|_| ()))
}

#[cfg(test)]
pub(crate) mod tests {
  use super::*;
  use crate::ops::{pointers, results};
  use crate::ops::results::Outcome;
  use crate::store::Store;
  use crate::store::changeset::{Change, change_to};

  // A service's store holding the core's tables and a `member` table.
  pub(crate) fn store(member: &str) -> (tempfile::TempDir, Store) {
    let directory = tempfile::tempdir().unwrap();
    let store = Store::open(&directory.path().join("identity.db")).unwrap();
    store.migrate(crate::channels::service::MIGRATIONS).unwrap();
    store.run_batch(member).unwrap();
    (directory, store)
  }

  pub(crate) const MEMBER: &str = "CREATE TABLE member (id TEXT PRIMARY KEY, role TEXT) WITHOUT ROWID";

  pub(crate) fn place(hash: &str, time: u64) -> Pointer {
    Pointer { hash: hash.into(), time, device: None }
  }

  fn member(id: &str) -> Row {
    Row { table: "member".into(), key: json!([id]).to_string() }
  }

  // Apply an op at a place as dispatch does: its writes and its result committed together.
  pub(crate) fn apply(store: &Store, at: &Pointer, sql: &str) {
    pointers::write(store, at).unwrap();
    store.begin_write().unwrap();
    store.run_batch(sql).unwrap();
    let changeset = store.changeset().unwrap();
    results::record(store, &at.hash, &Outcome::Applied { output: json!(null), calls: json!([]) }, &[], &changeset).unwrap();
    store.end_write(true).unwrap();
  }

  // Ann is granted a role at 10 and loses it at 30; Bob joins at 20, applied
  // last. The ops after a place that changed a row come newest first,
  // whatever order they applied in.
  #[test]
  fn the_ops_after_a_place_that_changed_a_row_come_newest_first() {
    let (_directory, store) = store(MEMBER);
    apply(&store, &place("grant", 10), "INSERT INTO member VALUES ('ann', 'inviter')");
    apply(&store, &place("revoke", 30), "UPDATE member SET role = NULL WHERE id = 'ann'");
    apply(&store, &place("add bob", 20), "INSERT INTO member VALUES ('bob', NULL)");
    let text = |value: &str| Some(rusqlite::types::Value::Text(value.into()));
    let revoked = Change::Updated(vec![text("ann"), text("inviter")]);
    let cases = [("ann", 5, vec![revoked.clone(), Change::Inserted]), ("ann", 15, vec![revoked]), ("ann", 35, vec![]), ("bob", 15, vec![Change::Inserted]), ("bob", 25, vec![])];
    for (id, time, expected) in cases {
      let changes: Vec<Change> = later_changesets(&store, &member(id), &place("reader", time)).unwrap().iter().map(|changeset| change_to(changeset, &member(id)).unwrap().unwrap()).collect();
      assert_eq!(changes, expected, "{id} after {time}");
    }
  }

  // However many ops wrote or read a row before a place, the ones after it
  // are one range of the index, never a scan of the row's whole history.
  #[test]
  fn the_writers_and_readers_of_a_row_after_a_place_are_one_range_of_their_index() {
    let (_directory, store) = store(MEMBER);
    let (after, params) = after(&member("ann"), &place("reader", 5), "hash");
    let plan = |sql: String| -> String { store.all(&format!("EXPLAIN QUERY PLAN {sql}"), &params).unwrap().iter().map(|step| step["detail"].as_str().unwrap().to_string()).collect::<Vec<_>>().join("; ") };
    let writers = plan(format!("SELECT hash FROM carbon_write WHERE {after}"));
    assert!(writers.contains("USING PRIMARY KEY (tbl=? AND key=? AND (time,device,hash)>(?,?,?))"), "{writers}");
    let (after, _) = super::after(&member("ann"), &place("reader", 5), "reader");
    let readers = plan(format!("SELECT reader FROM carbon_read WHERE {after}"));
    assert!(readers.contains("USING PRIMARY KEY (tbl=? AND key=? AND (time,device,reader)>(?,?,?))"), "{readers}");
  }

  #[test]
  fn readers_placed_after_a_change_to_a_row_they_read_are_found_once_each() {
    let (_directory, store) = store(MEMBER);
    let reading = |hash: &str, time: u64| Reading { place: place(hash, time), store: "books".into() };
    record(&store, &[member("ann"), member("bob")], &reading("early", 5)).unwrap();
    record(&store, &[member("ann"), member("bob")], &reading("late", 25)).unwrap();
    record(&store, &[member("carol")], &reading("other", 30)).unwrap();
    assert_eq!(readers_of(&store, &[member("ann"), member("bob")], &place("change", 20)).unwrap(), [reading("late", 25)]);
    forget(&store, &[reading("late", 25)]).unwrap();
    assert_eq!(readers_of(&store, &[member("ann")], &place("change", 20)).unwrap(), []);
  }
}
