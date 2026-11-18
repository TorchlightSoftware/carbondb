//! A channel on this device: one p2panda space, and one log every service in
//! it shares (`crate::ops::oplog`), beside the services' own stores. A
//! replicated channel's log is published to its space and takes in what its
//! peers publish; a local channel, the system, never replicates.
//!
//! A channel's files sit in its identity's directory: its log, the store of
//! its own `[channel]`, named by the channel's id, the store of its
//! `[@carbon]`, and one store per service it names. Which channels this device
//! holds is which logs are there; which services each holds follows from its
//! `[channel]`. The system's sit directly in the data directory.

use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::store::Store;
use carbondb_service_bindings::migrations::Migration;

// The tables of a channel's log, as forward-only migrations.
pub const MIGRATIONS: &[Migration] = &[("log/0001_channel_log", include_str!("../../migrations/log/0001_channel_log.sql"))];

// Whether a channel is synced with peers or is this device's own.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
  Replicated,
  Local,
}

pub struct Channel {
  pub id: String,
  // The identity whose channel this is on this device; none for the system.
  pub identity: Option<String>,
  pub kind: Kind,
  // Where its files sit.
  directory: PathBuf,
  log: Store,
}

impl Channel {
  // A channel's log in a directory, created on first open.
  pub fn open(directory: &Path, id: &str, identity: Option<&str>, kind: Kind) -> Result<Channel> {
    std::fs::create_dir_all(directory).map_err(|error| crate::error::bad_request(format!("Cannot create {}: {error}", directory.display())))?;
    let log = Store::open(&log_path(directory, id))?;
    log.migrate(MIGRATIONS)?;
    Ok(Channel { id: id.to_string(), identity: identity.map(str::to_string), kind, directory: directory.to_path_buf(), log })
  }

  // Its shared log.
  pub fn log(&self) -> &Store {
    &self.log
  }

  // Where one of its services' stores sits.
  pub fn store_path(&self, store: &str) -> PathBuf {
    self.directory.join(format!("{store}.db"))
  }

  // The id of its `[@carbon]`'s store.
  pub fn carbon(&self) -> String {
    format!("{}.carbon", self.id)
  }

  pub fn directory(&self) -> &Path {
    &self.directory
  }
}

// Where a channel's log sits in a directory.
pub fn log_path(directory: &Path, id: &str) -> PathBuf {
  directory.join(format!("{id}.log.db"))
}

// The channels whose logs a directory holds, by id.
pub fn held_in(directory: &Path) -> Vec<String> {
  let Ok(entries) = std::fs::read_dir(directory) else { return vec![] };
  entries.filter_map(|entry| entry.ok()?.file_name().to_str()?.strip_suffix(".log.db").map(str::to_string)).collect()
}
