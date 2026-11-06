//! Results: what this device did with each op in its op log, keyed by the op's
//! hash. An op applied commits its result with its writes; an op refused rolls
//! its writes back and keeps only its result, saying why. Results are this
//! device's own record and never leave it.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::error::{Result, ServiceError};
use crate::Db;

pub const SCHEMA: &str = include_str!("../migrations/results.sql");

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

pub fn record(db: &dyn Db, hash: &str, outcome: &Outcome) -> Result<()> {
  let (name, output, error, calls) = match outcome {
    Outcome::Applied { output, calls } => ("applied", Some(output.to_string()), None, calls),
    Outcome::Refused { error, calls } => ("refused", None, Some(json!(error).to_string()), calls),
  };
  db.run(
    "INSERT INTO carbon_result (hash, outcome, output, error, calls) VALUES (?, ?, ?, ?, ?)",
    &[json!(hash), json!(name), json!(output), json!(error), json!(calls.to_string())],
  )?;
  Ok(())
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
