//! Measure what a row read as of a place costs, at the kept WAL's snapshot and
//! rebuilt from changesets three ways, and what each keeps on disk.
//!
//! cargo run --release -- [ops] [members] [runs]

use std::time::Instant;

use changeset_reads::history::{MEMBER, Store};
use changeset_reads::{Rows, Scratch, Table, in_cte, in_temp_table, query};
use rusqlite::types::Value;

// One way of reading a row as of a place.
type Read<'a> = Box<dyn Fn() -> Rows + 'a>;

fn main() {
  let mut args = std::env::args().skip(1).map(|arg| arg.parse::<i64>().expect("a number"));
  let (ops, members, runs) = (args.next().unwrap_or(10_000), args.next().unwrap_or(1_000), args.next().unwrap_or(50) as usize);
  let directory = tempfile::tempdir().unwrap();
  let store = Store::create(&directory.path().join("members.db"), MEMBER).unwrap();
  let applying = Instant::now();
  store.apply(0, &format!("WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {members}) INSERT INTO member SELECT 'm' || i, 'Member ' || i, 'member', 0 FROM n")).unwrap();
  // Every op writes the busy row m1, and one other row in turn.
  for time in 1..=ops {
    let other = time % (members - 1) + 2;
    store.apply(time, &format!("UPDATE member SET role = 'r{time}', joined = {time} WHERE id = 'm1'; UPDATE member SET role = 'r{time}' WHERE id = 'm{other}'")).unwrap();
  }
  let applied = applying.elapsed();
  let (wal, changesets, snapshots) = store.sizes().unwrap();
  println!("{ops} ops on {members} members, applied in {:.2}s ({:.0} µs per op)\n", applied.as_secs_f64(), applied.as_micros() as f64 / ops as f64);
  println!("| Kept | Bytes | Per op |\n|---|---|---|");
  for (kept, bytes) in [("WAL, never checkpointed", wal as i64), ("changesets", changesets), ("start snapshots", snapshots)] {
    println!("| {kept} | {bytes} | {} |", bytes / (ops + 1));
  }

  let reader = store.reader().unwrap();
  let table = Table::of(&reader, "member").unwrap();
  let scratch = Scratch::new(&table).unwrap();
  let sql = "SELECT role, joined + 1 FROM member WHERE id = ?";
  let reads: Vec<(&str, i64)> = [0, 1, 10, 100, 1_000, ops].into_iter().filter(|later| *later <= ops).map(|later| ("m1", ops - later)).chain([("m500", ops - 100)]).collect();
  println!("\nMedian µs per read, of {runs}:\n");
  println!("| Row | Later writes | Kept WAL | Rebuilt, temp table | Rebuilt, scratch | Rebuilt, CTE |\n|---|---|---|---|---|---|");
  for (id, place) in reads {
    let key = [Value::Text(id.into())];
    let later: i64 = reader.query_row("SELECT count(*) FROM carbon_write WHERE key = ? AND time > ?", (format!("{key:?}"), place), |row| row.get(0)).unwrap();
    // In the order of the table's columns.
    let ways: [Read; 4] = [
      Box::new(|| store.read_at_snapshot(&reader, &key, place, sql, &key).unwrap()),
      Box::new(|| store.read_rebuilt(&reader, &table, &key, place, |row| in_temp_table(&reader, &table, row, sql, &key)).unwrap()),
      Box::new(|| store.read_rebuilt(&reader, &table, &key, place, |row| scratch.read(row, sql, &key)).unwrap()),
      Box::new(|| store.read_rebuilt(&reader, &table, &key, place, |row| in_cte(&reader, &table, row, sql, &key)).unwrap()),
    ];
    let answers: Vec<Rows> = ways.iter().map(|read| read()).collect();
    assert!(answers.iter().all(|answer| *answer == answers[0]), "{id} as of {place}: {answers:?}");
    let timings: Vec<String> = ways.iter().map(|read| format!("{:.0}", median(runs, read))).collect();
    println!("| {id} | {later} | {} |", timings.join(" | "));
  }
  let current = median(runs, &|| query(&reader, sql, &[Value::Text("m1".into())]).unwrap());
  println!("\nThe current row, read directly: {current:.0} µs");
}

// The median time a read takes, in microseconds.
fn median(runs: usize, read: &dyn Fn() -> Rows) -> f64 {
  let mut times: Vec<f64> = (0..runs)
    .map(|_| {
      let started = Instant::now();
      read();
      started.elapsed().as_secs_f64() * 1e6
    })
    .collect();
  times.sort_by(f64::total_cmp);
  times[runs / 2]
}
