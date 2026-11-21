//! Results: what this device did with each op in its op log, keyed by the op's
//! hash. An op applied commits its result with its writes; an op refused rolls
//! its writes back and keeps only its result, saying why. Each result also
//! keeps the op's read set and changeset, which rewind works from; the rows
//! its changeset wrote are indexed beside it by the op's place
//! (`carbon_write`), so a read as of a place finds the ops after it that wrote
//! a row, and undoes their changes to it (`ops::reads`). Results are this
//! device's own record and never leave it; a rewind forgets those of the ops
//! it undoes, which leaves them pending to run again.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use rusqlite::types::Value as Sql;

use crate::sql::authorizer::Read;
use crate::error::{Result, ServiceError};
use crate::store::changeset::rows;
use crate::store::{Store, to_sql};
use crate::Db;

// What became of an op, with the calls it made.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "lowercase")]
pub enum Outcome {
  Applied { output: Value, calls: Value },
  Refused { error: ServiceError, calls: Value },
}

impl Outcome {
  pub fn of(result: &Result<Value>, calls: Value) -> Outcome {
    match result {
      Ok(output) => Outcome::Applied { output: output.clone(), calls },
      Err(error) => Outcome::Refused { error: error.clone(), calls },
    }
  }
}

// Record what became of an op, with the tables and rows it read and the
// changeset it committed, kept as bytes; and index the rows its changeset
// wrote at the op's place, which its pointer holds.
pub fn record(store: &Store, hash: &str, outcome: &Outcome, read_set: &[Read], changeset: &[u8]) -> Result<()> {
  let (name, output, error, calls) = match outcome {
    Outcome::Applied { output, calls } => ("applied", Some(output.to_string()), None, calls),
    Outcome::Refused { error, calls } => ("refused", None, Some(json!(error).to_string()), calls),
  };
  let columns = [json!(hash), json!(name), json!(output), json!(error), json!(calls.to_string()), json!(json!(read_set).to_string())];
  let params: Vec<Sql> = columns.iter().map(to_sql).chain([Sql::Blob(changeset.to_vec())]).collect();
  store.run_typed("INSERT INTO carbon_result (hash, outcome, output, error, calls, read_set, changeset) VALUES (?, ?, ?, ?, ?, ?, ?)", &params)?;
  let written = "INSERT INTO carbon_write (tbl, key, time, device, hash) SELECT ?, ?, time, COALESCE(device, ''), hash FROM carbon_pointer WHERE hash = ?";
  rows(changeset)?.iter().try_for_each(|row| store.run(written, &[json!(row.table), json!(row.key), json!(hash)]).map(|_| ()))
}

// Forget the results of ops a rewind undid, and the rows they wrote, leaving them pending.
pub fn forget(db: &dyn Db, hashes: &[String]) -> Result<()> {
  hashes.iter().try_for_each(|hash| {
    db.run("DELETE FROM carbon_result WHERE hash = ?", &[json!(hash)])?;
    db.run("DELETE FROM carbon_write WHERE hash = ?", &[json!(hash)]).map(|_| ())
  })
}

// The result of an op, if it has one.
pub fn of(db: &dyn Db, hash: &str) -> Result<Option<Outcome>> {
  let row = db.get("SELECT outcome, output, error, calls FROM carbon_result WHERE hash = ?", &[json!(hash)])?;
  if row.is_null() {
    return Ok(None);
  }
  let parse = |column: &str| -> Result<Value> { Ok(serde_json::from_str(row[column].as_str().unwrap_or("null"))?) };
  Ok(Some(match row["outcome"].as_str() {
    Some("applied") => Outcome::Applied { output: parse("output")?, calls: parse("calls")? },
    _ => Outcome::Refused { error: serde_json::from_value(parse("error")?)?, calls: parse("calls")? },
  }))
}
