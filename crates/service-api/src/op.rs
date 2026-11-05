//! An op as a handler sees it: a service call that writes, stamped with who
//! made it and when. Its hash names it on every replica; everything a handler
//! derives from it, such as ids and randomness, comes from that hash.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Op {
  pub key: String,
  pub input: serde_json::Value,
  pub author: Option<String>,
  pub device: Option<String>,
  // The schema version: how many migrations the called service declares.
  pub version: usize,
  pub time: u64,
  pub hash: String,
}
