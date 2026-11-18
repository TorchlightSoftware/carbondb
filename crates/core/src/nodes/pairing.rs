//! Enrolling a device: the new device shows a code, and a device already in
//! the identity scans it (`POST [@carbon]/enroll {code, name, tags}`).
//!
//! The new device makes its key pair and code with `POST [@carbon]/devices`
//! on its system handle, and starts its node. The code carries its p2panda
//! member, p2panda's 305-byte encoding of its device key and key bundle,
//! cross-signed, a pairing secret, and its relay URL if it names one: all the
//! existing device needs to add it, without waiting on it.
//!
//! On the existing device, `[@carbon]` lists the new device in `[identity]`,
//! signed with its own device key; then its `enroll` effect has the
//! identity's node register the new device's key bundle and add it to every
//! space the node follows (`members`), and the call answers. The node then sends the new device,
//! sealed with the pairing secret on an ephemeral topic the secret names, the
//! identity and its channels. Ephemeral messages are not stored, so the new
//! device announces itself on the topic until that arrives, and the existing
//! device answers each announcement. When it does, the new device's core
//! records the identity by an op on its system handle, and the `ref` sync
//! mode loads the rest. A code expires after `EXPIRES`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use p2panda::spaces::Member;
use p2panda::{Hash, RelayUrl, Topic, VerifyingKey};
use p2panda_encryption::crypto::aead::{aead_decrypt, aead_encrypt};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio_stream::StreamExt;

use super::members::add_members;
use super::on_network;
use crate::calls::service_call::ServiceCall;
use crate::carbon::Carbon;
use crate::error::{Result, bad_request, forbidden, internal, single_node};
use crate::hash::to_hex;
use crate::keys;
use crate::services::carbon as local;
use crate::{HandlerContext, Reply, Routing};
use carbondb_service_bindings::hash::from_hex;

// How long a code can be scanned, and its devices look for each other.
pub const EXPIRES: Duration = Duration::from_secs(10 * 60);

// How often the new device announces itself on the pairing topic.
const ANNOUNCE_EVERY: Duration = Duration::from_millis(500);

const MEMBER_HEX: usize = p2panda_spaces::member::MEMBER_BYTES_LEN * 2;
const SECRET_HEX: usize = 64;

// What a code carries.
pub(crate) struct Code {
  pub member: Member,
  pub secret: [u8; 32],
  pub relay: Option<RelayUrl>,
}

impl Code {
  // The code as text: the member and the secret in hex, then the relay URL, if any.
  pub fn encode(&self) -> String {
    format!("{}{}{}", to_hex(&self.member.to_bytes()), to_hex(&self.secret), self.relay.as_ref().map_or_else(String::new, |relay| relay.to_string()))
  }

  pub fn decode(text: &str) -> Result<Code> {
    let malformed = || bad_request("Not a code a device showed");
    if text.len() < MEMBER_HEX + SECRET_HEX || !text.is_char_boundary(MEMBER_HEX + SECRET_HEX) {
      return Err(malformed());
    }
    let member = Member::from_bytes(&from_hex(&text[..MEMBER_HEX]).ok_or_else(malformed)?).map_err(|_| malformed())?;
    member.verify().map_err(|error| bad_request(format!("The code's device key and key bundle do not match: {error}")))?;
    let secret = from_hex(&text[MEMBER_HEX..MEMBER_HEX + SECRET_HEX]).and_then(|bytes| bytes.try_into().ok()).ok_or_else(malformed)?;
    let relay = match &text[MEMBER_HEX + SECRET_HEX..] {
      "" => None,
      relay => Some(RelayUrl::from_str(relay).map_err(|_| malformed())?),
    };
    Ok(Code { member, secret, relay })
  }

  // The device the code shows, as its key.
  pub fn device(&self) -> String {
    self.member.id().to_hex()
  }
}

// The ephemeral topic a pairing secret names.
fn topic(secret: &[u8; 32]) -> Topic {
  Topic::from(Hash::digest(secret))
}

// What travels on a pairing topic: the new device announcing itself, and the
// identity it joins, sealed with the pairing secret.
#[derive(Debug, Clone, Serialize, Deserialize)]
enum Pairing {
  Announce,
  Joined { nonce: String, sealed: String },
}

// The identity a device joins, as it records it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Joined {
  pub slug: String,
  pub id: String,
  pub channel: String,
  pub personal: String,
}

fn seal(secret: &[u8; 32], joined: &Joined) -> Result<Pairing> {
  let mut nonce = [0u8; 12];
  getrandom::fill(&mut nonce).map_err(|error| internal(format!("No randomness for a nonce: {error}")))?;
  let sealed = aead_encrypt(secret, &serde_json::to_vec(joined)?, nonce, None).map_err(|error| internal(format!("Cannot seal a pairing: {error}")))?;
  Ok(Pairing::Joined { nonce: to_hex(&nonce), sealed: to_hex(&sealed) })
}

fn open(secret: &[u8; 32], nonce: &str, sealed: &str) -> Option<Joined> {
  let nonce: [u8; 12] = from_hex(nonce)?.try_into().ok()?;
  let plain = aead_decrypt(secret, &from_hex(sealed)?, nonce, None).ok()?;
  serde_json::from_slice(&plain).ok()
}

// A device waiting to join an identity: its key pair's secret, and the
// identity once the device that scanned its code has sent it.
pub(crate) struct Waiting {
  pub secret: String,
  pub joined: Option<Joined>,
}

// The devices on this CarbonDB waiting to join an identity, by device key.
#[derive(Default)]
pub(crate) struct Pairings {
  waiting: RefCell<HashMap<String, Waiting>>,
}

impl Carbon {
  // `POST [@carbon]/devices`: a key pair for a new device and the code that
  // shows it, its node started; the device then waits on the pairing topic
  // for the identity it joins.
  pub(crate) fn show_code(self: &Rc<Self>, call: Rc<ServiceCall>) {
    if self.config.sync == crate::config::SyncMode::None {
      let _ = call.settle(Err(single_node("Enrolling a device")));
      return;
    }
    let pair = match keys::generate() {
      Ok(pair) => pair,
      Err(error) => {
        let _ = call.settle(Err(error));
        return;
      }
    };
    let mut secret = [0u8; 32];
    if let Err(error) = getrandom::fill(&mut secret) {
      let _ = call.settle(Err(internal(format!("No randomness for a pairing secret: {error}"))));
      return;
    }
    if let Err(error) = keys::credentials(&pair.secret).map(|credentials| self.spawn_node(&pair.public, credentials)) {
      let _ = call.settle(Err(error));
      return;
    }
    self.nodes.pairings.waiting.borrow_mut().insert(pair.public.clone(), Waiting { secret: pair.secret.clone(), joined: None });
    let (carbon, device, relay) = (Rc::clone(self), pair.public.clone(), self.config.network.relays.first().cloned());
    self.with_node(&pair.public, Box::new(move |node| {
      let node = match node {
        Ok(node) => node,
        Err(error) => {
          let _ = call.settle(Err(error));
          return;
        }
      };
      let me = node.clone();
      on_network(&carbon.clone(), async move { me.me().await.map_err(|error| internal(format!("This device's key bundle is not ready: {error}"))) }, move |member| {
        let code = member.map(|member| Code { member, secret, relay });
        if let Ok(code) = &code {
          carbon.await_joined(&device, node, code.secret);
        }
        let _ = call.settle(code.map(|code| json!({"code": code.encode(), "device": device})));
      });
    }));
  }

  // Announce this new device on its pairing topic until the identity it joins
  // arrives, then record it; or give up once the code expires.
  fn await_joined(self: &Rc<Self>, device: &str, node: std::sync::Arc<p2panda::Node>, secret: [u8; 32]) {
    let waiting = async move {
      let (publisher, mut subscription) = node.ephemeral_stream::<Pairing>(topic(&secret)).await.map_err(|error| internal(format!("Cannot open a pairing topic: {error}")))?;
      let mut announce = tokio::time::interval(ANNOUNCE_EVERY);
      let expired = tokio::time::sleep(EXPIRES);
      tokio::pin!(expired);
      loop {
        tokio::select! {
          _ = &mut expired => return Err(bad_request("The code expired before a device scanned it")),
          _ = announce.tick() => {
            let _ = publisher.publish(Pairing::Announce).await;
          }
          message = subscription.next() => match message {
            Some(message) => if let Pairing::Joined { nonce, sealed } = message.body() && let Some(joined) = open(&secret, nonce, sealed) {
              return Ok(joined);
            },
            None => return Err(internal("The pairing topic closed")),
          },
        }
      }
    };
    let (carbon, device) = (Rc::clone(self), device.to_string());
    on_network(self, waiting, move |joined| match joined {
      Ok(joined) => carbon.join(&device, joined),
      Err(error) => {
        carbon.nodes.pairings.waiting.borrow_mut().remove(&device);
        carbon.stop_node(&device);
        eprintln!("[carbondb] device {device} did not join an identity: {}", error.message);
      }
    });
  }

  // The identity a new device joins has arrived: record it by an op on the system handle.
  fn join(self: &Rc<Self>, device: &str, joined: Joined) {
    if let Some(waiting) = self.nodes.pairings.waiting.borrow_mut().get_mut(device) {
      waiting.joined = Some(joined.clone());
    }
    let input = json!({"slug": joined.slug, "device": device, "joining": device});
    let call = crate::Call::from_key("POST [@carbon]/identity").expect("a service key");
    let system = Rc::clone(self.system());
    self.submit(&system, crate::Actor::default(), call, input, Box::new(move |recorded| {
      if let Err(error) = recorded {
        eprintln!("[carbondb] a joined identity was not recorded: {}", error.message);
      }
    }));
  }

  // The device waiting to join, once its identity has arrived: taken, to record the identity.
  pub(crate) fn take_joined(&self, device: &str) -> Option<(String, Joined)> {
    let mut waiting = self.nodes.pairings.waiting.borrow_mut();
    let joined = waiting.get(device)?.joined.clone()?;
    waiting.remove(device).map(|waiting| (waiting.secret, joined))
  }

  // An identity's node, once the device that joined it has recorded it: it was the device's.
  pub(crate) fn node_joins(&self, device: &str, identity: &str) {
    self.rename_node(device, identity);
  }
}

// A device to enroll: the code it shows, and the name and tags it is listed with.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Enrolling {
  code: String,
  name: String,
  #[serde(default)]
  tags: Vec<String>,
}

impl Carbon {
  // `POST [@carbon]/enroll`, on the identity's handle: the device a code
  // shows is listed in `[identity]`, signed by the `sign` effect; then the
  // `enroll` effect has the identity's node register its key bundle, add it to
  // every space the node follows, and answer its announcements with the
  // identity until the code expires. Answers the device as listed. Every
  // replica of the identity's channel replays it with those effects recorded.
  pub(crate) fn enroll(self: &Rc<Self>, call: Rc<ServiceCall>) {
    if self.config.sync == crate::config::SyncMode::None {
      let _ = call.settle(Err(single_node("Enrolling a device")));
      return;
    }
    let request: Result<Enrolling> = serde_json::from_value(call.input.clone()).map_err(Into::into);
    let device = request.and_then(|request| Code::decode(&request.code).map(|code| (json!({"pubkey": code.device(), "name": request.name, "tags": request.tags}), request.code)));
    let (device, code) = match device {
      Ok(device) => device,
      Err(error) => {
        let _ = call.settle(Err(error));
        return;
      }
    };
    let caller = Rc::clone(&call);
    HandlerContext::effect(call, "sign", json!({"key": "POST [identity]/devices", "recorded": device}), Box::new(move |signature| {
      let listing = signature.map(|signature| json!({"pubkey": device["pubkey"], "name": device["name"], "tags": device["tags"], "signature": signature}));
      match listing {
        Ok(listing) => list_and_admit(caller, listing, code),
        Err(error) => {
          let _ = caller.settle(Err(error));
        }
      }
    }));
  }
}

// List the device in `[identity]`, then admit it through the identity's node.
fn list_and_admit(call: Rc<ServiceCall>, listing: Value, code: String) {
  let caller = Rc::clone(&call);
  HandlerContext::call(call, "POST [identity]/devices", listing, Box::new(move |listed| match listed {
    Ok(listed) => {
      let answering = Rc::clone(&caller);
      HandlerContext::effect(caller, "enroll", json!({"code": code}), Box::new(move |enrolled| {
        let _ = answering.settle(enrolled.map(|_| listed));
      }));
    }
    Err(error) => {
      let _ = caller.settle(Err(error));
    }
  }));
}

// The `enroll` effect, on the device that scanned a code: the identity's node
// registers the new device's key bundle and adds it to every space it
// follows, then answers the device's key; it then answers the device's
// announcements with the identity until the code expires.
pub(crate) fn enroll_effect(caller: Rc<ServiceCall>, input: Value, reply: Reply) {
  let carbon = Rc::clone(caller.carbon());
  let prepared = (|| -> Result<(Code, String, Pairing)> {
    let code = Code::decode(input["code"].as_str().unwrap_or_default())?;
    let identity = caller.actor.identity.clone().ok_or_else(|| forbidden("Enrolling a device needs an acting identity"))?;
    let held = local::acting(&carbon.system.store().reader()?, &identity)?;
    let sealed = seal(&code.secret, &Joined { slug: held.slug, id: held.id, channel: held.channel, personal: held.personal })?;
    Ok((code, identity, sealed))
  })();
  let (code, identity, sealed) = match prepared {
    Ok(prepared) => prepared,
    Err(error) => return reply(Err(error)),
  };
  let admitting = Rc::clone(&carbon);
  carbon.with_node(&identity.clone(), Box::new(move |node| {
    let node = match node {
      Ok(node) => node,
      Err(error) => return reply(Err(error)),
    };
    let (member, secret, relay, device) = (code.member.clone(), code.secret, code.relay.clone(), code.device());
    let (registering, spaces) = (Arc::clone(&node), admitting.spaces_of(&identity));
    let key_text = device.clone();
    let added = async move {
      let key = key_text.parse::<VerifyingKey>().map_err(|error| internal(format!("{key_text} is not a device key: {error}")))?;
      if let Some(relay) = relay {
        let _ = registering.insert_bootstrap(member.id(), relay).await;
      }
      registering.register_member(member).await.map_err(|error| internal(format!("Cannot register a device's key bundle: {error}")))?;
      for space in spaces {
        add_members(&space, &[key]).await?;
      }
      Ok(())
    };
    let then = Rc::clone(&admitting);
    on_network(&admitting, added, move |added: Result<()>| {
      let answered = added.is_ok();
      reply(added.map(|()| json!({"pubkey": device})));
      if answered {
        then.answer_announcements(node, secret, sealed, code.device());
      }
    });
  }));
}

impl Carbon {
  // Answer the new device's announcements on its pairing topic with the sealed identity, until the code expires.
  fn answer_announcements(self: &Rc<Self>, node: Arc<p2panda::Node>, secret: [u8; 32], sealed: Pairing, device: String) {
    let answer = async move {
      let (publisher, mut subscription) = node.ephemeral_stream::<Pairing>(topic(&secret)).await.map_err(|error| internal(format!("Cannot open a pairing topic: {error}")))?;
      let expired = tokio::time::sleep(EXPIRES);
      tokio::pin!(expired);
      loop {
        tokio::select! {
          _ = &mut expired => return Ok(()),
          message = subscription.next() => match message {
            Some(message) if matches!(message.body(), Pairing::Announce) && message.author().to_hex() == device => {
              let _ = publisher.publish(sealed.clone()).await;
            }
            Some(_) => {}
            None => return Ok(()),
          },
        }
      }
    };
    on_network(self, answer, |_: Result<()>| {});
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn a_joined_identity_opens_only_with_its_pairing_secret() {
    let joined = Joined { slug: "alice".into(), id: "i".into(), channel: "c".into(), personal: "p".into() };
    let Pairing::Joined { nonce, sealed } = seal(&[7; 32], &joined).unwrap() else { panic!("sealed") };
    assert_eq!(open(&[7; 32], &nonce, &sealed), Some(joined));
    assert_eq!(open(&[8; 32], &nonce, &sealed), None);
  }

  #[test]
  fn refuses_text_that_is_not_a_code() {
    for text in ["", "zz", &"0".repeat(MEMBER_HEX + SECRET_HEX)] {
      assert_eq!(Code::decode(text).err().map(|error| error.status), Some(400), "{text}");
    }
  }
}
