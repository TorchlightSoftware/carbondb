use super::*;
use p2panda_encryption::traits::KeyBundle;
use crate::testing::{awaited, found, reopen, replicated, texts, until};

// An identity's node, once it has started.
fn node(carbon: &Rc<crate::Carbon>, identity: &str) -> std::sync::Arc<p2panda::Node> {
  awaited(|ready| carbon.with_node(identity, ready)).unwrap()
}

#[test]
fn each_identity_runs_a_node_signing_with_its_device_key_across_restarts() {
  let (directory, carbon) = replicated();
  let identity = alice(&carbon);
  let started = node(&carbon, &identity.id);
  assert_eq!(started.id().to_hex(), identity.device);
  let identity_key = *crate::nodes::runtime().block_on(started.me()).unwrap().key_bundle().identity_key();
  assert!(carbon.node_path(&identity.device).exists());
  drop((started, carbon));
  let carbon = reopen(&directory);
  let restarted = node(&carbon, &identity.id);
  assert_eq!(restarted.id().to_hex(), identity.device);
  until(|| [&identity.channel, &identity.personal].iter().all(|space| carbon.followed(space).is_some()));
  assert_eq!(*crate::nodes::runtime().block_on(restarted.me()).unwrap().key_bundle().identity_key(), identity_key);
}

#[test]
fn each_identity_has_a_node_of_its_own() {
  let (_directory, carbon) = replicated();
  let identities: Vec<_> = ["alice", "bob"].iter().map(|slug| found(&carbon, slug, &[])).collect();
  let ids: Vec<String> = identities.iter().map(|identity| node(&carbon, &identity.id).id().to_hex()).collect();
  assert_eq!(ids, identities.iter().map(|identity| identity.device.clone()).collect::<Vec<_>>());
  assert_ne!(ids[0], ids[1]);
}

#[test]
fn a_single_node_carbondb_runs_no_node() {
  let (_directory, carbon) = carbon();
  let identity = alice(&carbon);
  assert_eq!(awaited(|ready| carbon.with_node(&identity.id, ready)).unwrap_err().status, 404);
}

#[test]
fn publishes_each_op_this_device_applied_in_a_synced_channel_once() {
  let (_directory, carbon) = replicated();
  let identity = alice(&carbon);
  let channel = carbon.channel(&identity.channel).unwrap();
  let hashes = |sql: &str| -> Vec<String> { channel.log().reader().unwrap().all(sql, &[]).unwrap().iter().map(|row| row["hash"].as_str().unwrap().to_string()).collect() };
  let applied = || hashes("SELECT hash FROM carbon_outcome WHERE outcome = 'applied' ORDER BY hash");
  let published = || hashes("SELECT hash FROM carbon_published ORDER BY hash");
  crate::testing::until(|| !applied().is_empty() && published() == applied());
  assert_eq!(identity.channel.len(), 64, "the own channel's id is its p2panda space's");
  assert!(carbon.followed(&identity.personal).is_some());
}

#[test]
fn a_failed_carbon_op_stops_following_the_p2panda_spaces_it_created() {
  let (directory, carbon) = replicated();
  let identity = alice(&carbon);
  let followed = || carbon.nodes.spaces.followed_ids().len();
  let held = || std::fs::read_dir(directory.path().join("data").join(&identity.id)).unwrap().count();
  let (before, files) = (followed(), held());
  let unusable = awaited(|answer| carbon.call(Some(&identity.personal), Some(&identity.id), "POST [@carbon]/channel", json!({"name": "!!"}), answer)).unwrap_err();
  assert_eq!(unusable.status, 400, "its name gives no slug, so its ref is refused after the channel was created");
  assert_eq!((followed(), held()), (before, files), "the channel and its p2panda space are gone");
}

// A message as it would arrive on a space of alice's, made by `device` for
// alice at `time`, from the p2panda author given; `n` names its operation.
fn message(space: &str, identity: &crate::Identity, device: &str, author: &str, time: u64, n: u8) -> crate::nodes::spaces::Arrival {
  let actor = Actor { identity: Some(identity.id.clone()), device: Some(device.to_string()) };
  let call = <crate::Call as Routing>::from_key("POST [notes]/run").unwrap();
  let op = crate::ops::op::stamp(&actor, &call, &json!({"do": [{"insert": format!("note {n}")}]}), "notes-v1", time);
  crate::nodes::spaces::Arrival { space: space.to_string(), operation: p2panda::Hash::digest([n]), author: author.to_string(), row: oplog::LogRow::new(op), own: false, signed: "{}".into() }
}

// The same message with its input changed after it was stamped.
fn forged(honest: crate::nodes::spaces::Arrival) -> crate::nodes::spaces::Arrival {
  let op = honest.row.op.clone();
  crate::nodes::spaces::Arrival { row: oplog::LogRow::new(crate::Op::new(op.key, json!({"do": []}), op.author, op.device, op.version, op.time, op.hash)), ..honest }
}

#[test]
fn a_message_that_fails_the_check_is_kept_as_evidence_and_one_not_yet_admissible_is_held() {
  let (_directory, carbon, timers) = crate::testing::syncing_timed("mode = \"ref\"\nmdns = \"off\"");
  let identity = alice(&carbon);
  let notes = crate::testing::personal_notes(&carbon, &identity);
  let personal = identity.personal.clone();
  until(|| carbon.followed(&personal).is_some());
  let (stranger, now) = (crate::keys::generate().unwrap().public, crate::ops::clock::wall());
  let log = carbon.channel(&personal).unwrap();
  let failed = || crate::ops::errors::all(log.log()).unwrap().into_iter().map(|evidence| evidence.failed).collect::<std::collections::BTreeSet<_>>();
  let held = || carbon.followed(&personal).unwrap().held.borrow().len();

  carbon.arrived(message(&personal, &identity, &identity.device, &stranger, now, 1));
  carbon.arrived(forged(message(&personal, &identity, &identity.device, &identity.device, now, 2)));
  assert_eq!(failed(), ["device is not the p2panda author", "hash"].map(String::from).into());

  carbon.arrived(message(&personal, &identity, &identity.device, &identity.device, now + 10 * 60 * 1000, 3));
  carbon.arrived(message(&personal, &identity, &stranger, &stranger, now, 4));
  assert_eq!(held(), 2, "one too early, one from a device alice does not list");
  timers.pass(crate::nodes::receive::UNLISTED_FOR);
  assert!(failed().contains("unlisted device"));
  assert_eq!(held(), 1, "the early one waits for the clock");
  assert_eq!(texts(notes.store()).unwrap(), json!([]), "none of them applied");
}
