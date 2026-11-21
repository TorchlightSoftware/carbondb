//! Consistent reads across services in a channel: a `callsync` read answers
//! each row as it stood at the reader's place, whatever order the service read
//! applied its ops in; it waits only for the service it reads; it refuses to
//! read more than one row by key; and an op placed before the reader that
//! changes a row it read runs it again (stories/backlog/consistent-channel-reads.md).

use super::*;
use crate::testing::{hung, register_script};

// A room running `[roles]`, `[books]` and `[ledger]` beside `[notes]`.
fn books_and_roles() -> (tempfile::TempDir, Rc<crate::Carbon>, Rc<crate::ServiceStore>, Rc<crate::ServiceStore>) {
  let (directory, carbon) = carbon();
  let identity = alice(&carbon);
  let room = room_channel(&carbon, &identity);
  for name in ["roles", "books", "ledger"] {
    register_script(&carbon, name);
    on(&carbon, &room, &identity, "POST [@carbon]/services", json!({"name": name})).unwrap();
  }
  let (roles, books) = (service(&carbon, &room, "roles"), service(&carbon, &room, "books"));
  (directory, carbon, roles, books)
}

// Give someone a role in `[roles]`: their row, keyed by their name.
fn role(who: &str, role: &str) -> Value {
  json!([{"set": [who, role]}])
}

// Post an entry in `[books]`, only when sarah reads as the bookkeeper in `[roles]`.
fn entry() -> Value {
  json!([{"callsync": "GET [roles]/run", "input": {"do": [{"get": "sarah"}]}, "expect": ["bookkeeper"]}, {"insert": "entry"}])
}

#[test]
fn a_callsync_read_answers_as_of_the_readers_place_and_runs_again_when_an_earlier_op_changes_it() {
  let grant = || role("sarah", "bookkeeper");
  let revoke = || role("sarah", "none");
  let cases = [
    ("the service read is already past the reader's place", vec![("roles", grant(), 1), ("roles", revoke(), 20), ("books", entry(), 10)], vec![Ok(())]),
    ("a late revocation placed before it refuses it", vec![("roles", grant(), 1), ("books", entry(), 10), ("roles", revoke(), 5)], vec![Err(409)]),
    ("a late revocation placed after it leaves it applied", vec![("roles", grant(), 1), ("books", entry(), 10), ("roles", revoke(), 15)], vec![Ok(())]),
    ("a late grant placed before it lets it through", vec![("books", entry(), 10), ("roles", grant(), 5)], vec![Ok(())]),
    ("a late change to another row leaves it applied", vec![("roles", grant(), 1), ("books", entry(), 10), ("roles", role("bob", "none"), 5)], vec![Ok(())]),
  ];
  for (name, arrivals, expected) in cases {
    let (_directory, carbon, roles, books) = books_and_roles();
    for (to, script, time) in arrivals {
      arrives(&carbon, if to == "roles" { &roles } else { &books }, script, time);
    }
    assert_eq!(outcomes(&books), expected, "{name}");
    let posted = if expected == [Ok(())] { json!(["entry"]) } else { json!([]) };
    assert_eq!(texts(&books.store().reader().unwrap()).unwrap(), posted, "{name}");
  }
}

// Ann holds the inviter role, and Dave adds Bob from a laptop that is offline,
// then revokes Ann's role from his phone. Ann's invitation, placed between
// them, reads both rows: no commit of `[roles]` holds Ann as an inviter with
// Bob a member, yet once Bob's addition arrives the invitation goes through.
#[test]
fn a_read_of_two_rows_finds_each_as_it_stood_at_the_readers_place_though_no_commit_held_both() {
  let (_directory, carbon, roles, books) = books_and_roles();
  let invitation = json!([{"callsync": "GET [roles]/run", "input": {"do": [{"get": "ann"}, {"get": "bob"}]}, "expect": ["inviter", "member"]}, {"insert": "invitation"}]);
  arrives(&carbon, &roles, role("ann", "inviter"), 1);
  arrives(&carbon, &roles, role("ann", "none"), 30);
  arrives(&carbon, &books, invitation, 25);
  assert_eq!(outcomes(&books), vec![Err(409)], "Bob is not a member yet");
  arrives(&carbon, &roles, role("bob", "member"), 20);
  assert_eq!(outcomes(&roles), vec![Ok(()), Ok(()), Ok(())]);
  assert_eq!(outcomes(&books), vec![Ok(())], "the invitation runs again as of its place, and goes through");
}

#[test]
fn a_read_through_callsync_reads_one_row_by_key_and_a_search_goes_through_call() {
  let cases = [("callsync", Err(400)), ("call", Ok(()))];
  for (binding, expected) in cases {
    let (_directory, carbon, roles, books) = books_and_roles();
    arrives(&carbon, &roles, role("sarah", "bookkeeper"), 1);
    arrives(&carbon, &books, json!([{binding: "GET [roles]/run", "input": {"do": [{"read": true}]}}]), 10);
    assert_eq!(outcomes(&books), vec![expected], "{binding}");
  }
}

#[test]
fn a_callsync_read_made_inside_a_callsync_read_is_as_of_the_first_callers_place() {
  let (_directory, carbon, roles, books) = books_and_roles();
  arrives(&carbon, &roles, role("sarah", "bookkeeper"), 1);
  arrives(&carbon, &roles, role("sarah", "none"), 20);
  let through_ledger = json!([{"callsync": "GET [ledger]/run", "input": {"do": [{"callsync": "GET [roles]/run", "input": {"do": [{"get": "sarah"}]}}]}, "expect": [["bookkeeper"]]}, {"insert": "entry"}]);
  arrives(&carbon, &books, through_ledger, 10);
  assert_eq!(outcomes(&books), vec![Ok(())]);
}

#[test]
fn a_reader_waits_only_for_the_service_it_reads_to_apply_the_ops_before_its_place() {
  let (_directory, carbon, roles, books) = books_and_roles();
  arrives(&carbon, &roles, role("sarah", "bookkeeper"), 1);
  arrives(&carbon, &roles, json!([{"set": ["sarah", "none"]}, {"hang": true}]), 5);
  arrives(&carbon, &books, entry(), 10);
  assert_eq!(outcomes(&books), vec![Err(0)], "the entry waits for the revocation placed before it");
  let notes = service(&carbon, &books.channel.id, "notes");
  arrives(&carbon, &notes, json!([{"insert": "chat"}]), 7);
  assert_eq!(outcomes(&notes), vec![Ok(())], "a service that reads no other never waits");
  for revocation in hung() {
    revocation.settle(Ok(json!([]))).unwrap();
  }
  assert_eq!(outcomes(&books), vec![Err(409)], "then reads Sarah's role as of its place, revoked");
}

// `[books]`' entry at 10 waits to read `[roles]` until its op at 5 applies;
// that op writes into `[books]`, where it would queue behind the entry
// waiting on it. The write is refused at once instead, and the entry goes on.
#[test]
fn a_write_into_a_store_whose_op_waits_to_read_the_writer_is_refused_at_once() {
  let (_directory, carbon, roles, books) = books_and_roles();
  arrives(&carbon, &roles, role("sarah", "bookkeeper"), 1);
  arrives(&carbon, &roles, json!([{"hang": true}]), 3);
  arrives(&carbon, &roles, json!([{"call": "POST [books]/run", "input": {"do": [{"insert": "from roles"}]}}]), 5);
  arrives(&carbon, &books, entry(), 10);
  assert_eq!(outcomes(&books), vec![Err(0)], "the entry waits for the ops placed before it");
  for busy in hung() {
    busy.settle(Ok(json!([]))).unwrap();
  }
  assert_eq!(outcomes(&roles), vec![Ok(()), Ok(()), Err(508)], "the write back into [books] would wait for itself");
  assert_eq!(outcomes(&books), vec![Ok(())]);
}
