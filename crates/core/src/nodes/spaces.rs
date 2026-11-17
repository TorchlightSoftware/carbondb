//! The p2panda space of each service space a node syncs. Its messages are
//! ops as their log rows hold them, with the effects their first run
//! recorded. A space's p2panda space is created with it, by the `create`
//! effect, or joined when this device loads it; either way it is followed:
//! what arrives on it reaches the core's thread, which acknowledges each
//! message once it has dealt with it (`crate::nodes::receive`), and this
//! device's applied ops are published to it from the op log (`publish`).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::str::FromStr;
use std::sync::Arc;

use p2panda::spaces::Space;
use p2panda::streams::{ProcessedOperation, Source, StreamEvent};
use p2panda::Hash;
use serde_json::json;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio_stream::StreamExt;

use super::on_network;
use crate::carbon::Carbon;
use crate::error::{Result, bad_request, internal};
use crate::hash::to_hex;
use crate::ops::oplog::LogRow;
use crate::Db;

// A service space's p2panda space, as the core holds it: the space, the
// identity whose node syncs it and this device's key in it, where
// acknowledgements go, and what is under way.
pub(crate) struct Followed {
  pub space: Arc<Space<LogRow>>,
  // The identity whose node syncs it.
  pub identity: String,
  // This device's key in the identity, its node's id: the device whose ops it publishes.
  pub device: String,
  acks: UnboundedSender<Hash>,
  // The ops p2panda is publishing now, by op hash.
  pub(crate) publishing: RefCell<HashSet<String>>,
  // Messages held unacknowledged, to be offered again (`crate::nodes::receive`).
  pub(crate) held: RefCell<Vec<Held>>,
  // What waits to be written to the space's file until no op applies there.
  pub(crate) unrecorded: RefCell<Vec<Unrecorded>>,
  // The devices being added as members now (`crate::nodes::members`).
  pub(crate) adding: RefCell<HashSet<String>>,
}

// A message held unacknowledged, and whether it fails once held too long, its device still unlisted.
pub(crate) struct Held {
  pub arrival: Arrival,
  pub expiring: bool,
}

// What the core writes to a space's file for its p2panda space, between ops.
pub(crate) enum Unrecorded {
  Published(String),
  Error(crate::ops::errors::Evidence),
}

impl Followed {
  // Acknowledge a message, so p2panda does not offer it again.
  pub(crate) fn ack(&self, operation: Hash) {
    let _ = self.acks.send(operation);
  }
}

// A message that arrived on a p2panda space, for the core: the p2panda
// operation it came in, its verified p2panda author, the op it carries,
// whether this device published it, and the signed operation kept as
// evidence should it fail the check.
pub(crate) struct Arrival {
  pub space: String,
  pub operation: Hash,
  pub author: String,
  pub row: LogRow,
  pub own: bool,
  pub signed: String,
}

// The p2panda spaces followed, by service space id, and the way messages reach the core.
pub(crate) struct Spaces {
  followed: RefCell<HashMap<String, Rc<Followed>>>,
  // The spaces being joined now.
  joining: RefCell<HashSet<String>>,
  arrivals: UnboundedSender<Arrival>,
  arriving: RefCell<Option<UnboundedReceiver<Arrival>>>,
}

impl Default for Spaces {
  fn default() -> Spaces {
    let (arrivals, arriving) = unbounded_channel();
    Spaces { followed: RefCell::default(), joining: RefCell::default(), arrivals, arriving: RefCell::new(Some(arriving)) }
  }
}

impl Spaces {
  // The service spaces whose p2panda spaces are followed.
  pub(crate) fn followed_ids(&self) -> Vec<String> {
    self.followed.borrow().keys().cloned().collect()
  }
}

impl Carbon {
  // Take what arrives on every followed space on the core's thread, as long as this CarbonDB is open.
  pub(crate) fn take_arrivals(self: &Rc<Self>) {
    let Some(mut arriving) = self.nodes.spaces.arriving.borrow_mut().take() else { return };
    let carbon = Rc::downgrade(self);
    let _ = self.spawn(Box::pin(async move {
      while let Some(arrival) = arriving.recv().await {
        let Some(carbon) = carbon.upgrade() else { return };
        carbon.arrived(arrival);
      }
    }));
  }

  // A service space's p2panda space, when this device follows it.
  pub(crate) fn followed(&self, space: &str) -> Option<Rc<Followed>> {
    self.nodes.spaces.followed.borrow().get(space).cloned()
  }

  // Create the p2panda space of a new service space, by the node of the
  // identity creating it, and follow it: its p2panda id is the space's id.
  pub(crate) fn create_p2panda_space(self: &Rc<Self>, identity: &str, created: Box<dyn FnOnce(Result<String>)>) {
    let (carbon, identity) = (Rc::clone(self), identity.to_string());
    self.with_node(&identity.clone(), Box::new(move |node| {
      let node = match node {
        Ok(node) => node,
        Err(error) => return created(Err(error)),
      };
      let device = node.id().to_hex();
      on_network(&carbon.clone(), async move { node.create_space::<LogRow>().await.map_err(|error| internal(format!("Cannot create a p2panda space: {error}"))) }, move |made| {
        created(made.map(|(space, subscription)| {
          let id = space.id().to_hex();
          carbon.follow(&id, &identity, &device, space, subscription);
          id
        }))
      });
    }));
  }

  // Join the p2panda space of a service space this device holds, by the node
  // of the identity that syncs it, and follow it once this device's node is
  // a member, which for a space it created is at once.
  pub(crate) fn join_p2panda_space(self: &Rc<Self>, space: &str, identity: &str) {
    if self.followed(space).is_some() || !self.nodes.spaces.joining.borrow_mut().insert(space.to_string()) {
      return;
    }
    let id = match Hash::from_str(space) {
      Ok(id) => id,
      Err(error) => {
        self.nodes.spaces.joining.borrow_mut().remove(space);
        return self.space_failed(space, bad_request(format!("{space} is not a p2panda space id: {error}")));
      }
    };
    let (carbon, space, identity) = (Rc::clone(self), space.to_string(), identity.to_string());
    self.with_node(&identity.clone(), Box::new(move |node| {
      let node = match node {
        Ok(node) => node,
        Err(error) => {
          carbon.nodes.spaces.joining.borrow_mut().remove(&space);
          return carbon.space_failed(&space, error);
        }
      };
      let device = node.id().to_hex();
      let joining = async move {
        let (pending, subscription) = node.space::<LogRow>(id).await.map_err(|error| internal(format!("Cannot subscribe to a p2panda space: {error}")))?;
        let joined = pending.joined().await.map_err(|error| internal(format!("Cannot join a p2panda space: {error}")))?;
        Ok((joined, subscription))
      };
      on_network(&carbon.clone(), joining, move |joined| {
        carbon.nodes.spaces.joining.borrow_mut().remove(&space);
        match joined {
          Ok((joined, subscription)) => carbon.follow(&space, &identity, &device, joined, subscription),
          Err(error) => carbon.space_failed(&space, error),
        }
      });
    }));
  }

  // Follow a p2panda space: what arrives on it goes to the core from a task
  // on the nodes' runtime, which acknowledges each message when the core says.
  fn follow(self: &Rc<Self>, space: &str, identity: &str, device: &str, joined: Space<LogRow>, subscription: p2panda::spaces::SpaceSubscription<LogRow>) {
    let (acks, acking) = unbounded_channel();
    let followed = Rc::new(Followed { space: Arc::new(joined), identity: identity.to_string(), device: device.to_string(), acks, publishing: RefCell::default(), held: RefCell::default(), unrecorded: RefCell::default(), adding: RefCell::default() });
    self.nodes.spaces.followed.borrow_mut().insert(space.to_string(), followed);
    super::runtime().spawn(forward(space.to_string(), subscription, self.nodes.spaces.arrivals.clone(), acking));
    self.publish(space);
    self.add_listed_devices(space);
  }

  // A p2panda space this device could not follow: its service space is not synced, and the failure is reported.
  fn space_failed(&self, space: &str, error: crate::ServiceError) {
    eprintln!("[carbondb] {space} is not synced: {}", error.message);
  }

  // Record which identity's node syncs a space this device holds.
  pub(crate) fn synced_by(&self, space: &str, identity: &str) -> Result<()> {
    self.system.store().run("UPDATE carbon_system_space SET identity = ? WHERE id = ?", &[json!(identity), json!(space)])?;
    Ok(())
  }

  // Follow again the p2panda spaces an identity's node syncs, once it has started.
  pub(crate) fn rejoin(self: &Rc<Self>, identity: &str) -> Result<()> {
    let held = self.system.store().reader()?.all("SELECT id FROM carbon_system_space WHERE identity = ?", &[json!(identity)])?;
    held.iter().filter_map(|row| row["id"].as_str()).for_each(|space| self.join_p2panda_space(space, identity));
    Ok(())
  }
}

// Forward what arrives on a p2panda space to the core, keeping each message
// until the core says to acknowledge it.
async fn forward(space: String, mut subscription: p2panda::spaces::SpaceSubscription<LogRow>, arrivals: UnboundedSender<Arrival>, mut acking: UnboundedReceiver<Hash>) {
  let mut unacknowledged: HashMap<Hash, ProcessedOperation<LogRow>> = HashMap::new();
  loop {
    tokio::select! {
      event = subscription.next() => match event {
        Some(StreamEvent::Processed { operation, source }) => {
          let arrival = Arrival { space: space.clone(), operation: operation.id(), author: operation.author().to_hex(), row: operation.message().clone(), own: matches!(source, Source::Publish), signed: evidence(&operation) };
          unacknowledged.insert(operation.id(), operation);
          if arrivals.send(arrival).is_err() {
            return;
          }
        }
        Some(_) => {}
        None => return,
      },
      acked = acking.recv() => match acked {
        Some(id) => {
          if let Some(operation) = unacknowledged.remove(&id) {
            let _ = operation.ack().await;
          }
        }
        None => return,
      },
    }
  }
}

// A p2panda operation as evidence: its signed header and its body, in hex.
fn evidence(operation: &ProcessedOperation<LogRow>) -> String {
  let signed = &operation.processed().operation;
  json!({"header": to_hex(&signed.header.encode()), "body": signed.body.as_ref().map(|body| to_hex(&body.to_bytes()))}).to_string()
}
