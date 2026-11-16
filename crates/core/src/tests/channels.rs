use super::*;

#[test]
fn creates_a_channel_inside_another_and_points_to_it() {
  let (_directory, carbon) = carbon();
  let identity = alice(&carbon);
  let create = |name: &str| on(&carbon, &identity.personal, &identity, "POST [channel]/channel", json!({"name": name})).unwrap();
  let created = create("Randall's Playhouse");
  assert_eq!((created["slug"].as_str(), created["kind"].as_str()), (Some("randalls-playhouse"), Some("channel")));
  assert_eq!(create("Randall's Playhouse")["slug"].as_str(), Some("randalls-playhouse-2"));
  let channel = created["channel"].as_str().unwrap();
  assert_eq!(carbon.find_ref(&identity.personal, "channel", "randalls-playhouse").unwrap()["channel"], json!(channel));

  let playhouse = space_now(&carbon, channel).unwrap();
  assert_eq!(logged(&playhouse), vec!["POST [channel]/origin"]);
  assert_eq!(playhouse.runs.channel.as_deref(), Some(channel), "a channel's space belongs to itself");
  let status = on(&carbon, channel, &identity, "GET [@carbon]/status", Value::Null).unwrap();
  assert_eq!(
    (&status["name"], &status["kind"], &status["access"], &status["sync"], &status["peers"], &status["services"]),
    (&json!("Randall's Playhouse"), &json!("channel"), &json!(["manage"]), &json!("off"), &json!([]), &json!([]))
  );
  assert_eq!(status["state"], json!("idle"));
}

#[test]
fn reports_the_status_of_a_channel_or_one_of_its_services_for_the_identity_acting() {
  let (_directory, carbon) = carbon();
  let identity = alice(&carbon);
  let room = room_channel(&carbon, &identity);
  let channel = on(&carbon, &room, &identity, "GET [@carbon]/status", Value::Null).unwrap();
  assert_eq!(channel["services"], json!([{"name": "notes", "access": ["manage"], "sync": "off", "state": "idle"}]));
  let notes = on(&carbon, &room, &identity, "GET [@carbon]/status", json!({"service": "notes"})).unwrap();
  assert_eq!(notes, json!({"name": "notes", "access": ["manage"], "sync": "off", "peers": [], "state": "idle"}));
  let stranger = now(|answer| carbon.call(Some(&room), None, "GET [@carbon]/status", Value::Null, answer)).unwrap();
  assert_eq!(stranger["access"], json!([]), "no one acting has no access, and no identity's access is borrowed");
  let refused = [("GET [@carbon]/status", json!({"service": "chat"}), 404), ("GET [@carbon]/status", json!("notes"), 400)];
  for (key, input, status) in refused {
    assert_eq!(on(&carbon, &room, &identity, key, input.clone()).unwrap_err().status, status, "{input}");
  }
  let system = call_now(&carbon, carbon.system(), &actor(&identity), "GET [@carbon]/status", json!({})).unwrap_err();
  assert_eq!(system.message, "Make this call on a channel's handle");
}

#[test]
fn names_a_service_it_creates_and_resolves_calls_to_it() {
  let (_directory, carbon) = carbon();
  crate::testing::register_script(&carbon, "jots");
  let identity = alice(&carbon);
  let room = room_channel(&carbon, &identity);
  let named = on(&carbon, &room, &identity, "POST [channel]/services", json!({"name": "scratch", "package": "jots"})).unwrap();
  assert_eq!(named["version"], json!("jots-v1"));
  on(&carbon, &room, &identity, "POST [scratch]/run", json!({"do": [{"insert": "s"}]})).unwrap();
  assert_eq!(on(&carbon, &room, &identity, "GET [scratch]/texts", Value::Null).unwrap(), json!(["s"]));
  assert_eq!(texts(&service(&carbon, &room, "notes").store().reader().unwrap()).unwrap(), json!([]), "each service keeps its own tables");
  let services: Vec<Value> = on(&carbon, &room, &identity, "GET [channel]/services", Value::Null).unwrap().as_array().unwrap().iter().map(|named| named["name"].clone()).collect();
  assert_eq!(services, [json!("notes"), json!("scratch")]);
  let refused = [
    (json!({"name": "notes"}), 409),
    (json!({"name": "channel"}), 409),
    (json!({"name": "crm"}), 404),
  ];
  for (input, status) in refused {
    assert_eq!(on(&carbon, &room, &identity, "POST [channel]/services", input.clone()).unwrap_err().status, status, "{input}");
  }
  assert_eq!(on(&carbon, &room, &identity, "GET [elsewhere]/texts", Value::Null).unwrap_err().status, 404);
}

#[test]
fn only_a_manager_changes_a_channel() {
  let (_directory, carbon) = carbon();
  let identity = alice(&carbon);
  let personal = space_now(&carbon, &identity.personal).unwrap();
  let stranger = Actor { identity: Some("someone-else".into()), device: None };
  let error = call_now(&carbon, &personal, &stranger, "POST [channel]/channel", json!({"name": "Room"})).unwrap_err();
  assert_eq!(error.status, 403);
  let leaving = call_now(&carbon, &personal, &stranger, "POST [channel]/leave", json!({})).unwrap_err();
  assert_eq!(leaving.code, "UNREACHABLE", "leaving needs no manage, only peers");
  assert_eq!(on(&carbon, &identity.personal, &identity, "GET [channel]/refs", Value::Null).unwrap(), json!([]));
}

#[test]
fn refuses_what_needs_peers_and_what_breaks_the_rules() {
  let (_directory, carbon) = carbon();
  let identity = alice(&carbon);
  let alice_on = |key: &str, input: Value| on(&carbon, &identity.personal, &identity, key, input).unwrap_err().code;
  let cases = [
    ("POST [channel]/members", json!({"contact": "sally", "access": "write"}), "UNREACHABLE"),
    ("POST [channel]/join", json!({"code": "scanned"}), "UNREACHABLE"),
    ("POST [channel]/leave", json!({}), "UNREACHABLE"),
    ("POST [channel]/manage", json!({"add": ["sally"]}), "UNREACHABLE"),
    ("DELETE [channel]/members/sally", Value::Null, "UNREACHABLE"),
    ("DELETE [channel]/invitations/i1", Value::Null, "UNREACHABLE"),
    ("POST [channel]/origin", json!({"kind": "team", "name": "x", "manage": []}), "CONFLICT"),
    ("POST [channel]/ref", json!({"channel": "c1"}), "BAD_REQUEST"),
    ("POST [channel]/ref", json!({"channel": "c1", "kind": "team", "name": "!!"}), "BAD_REQUEST"),
    ("GET [channel]/nothing", Value::Null, "NOT_FOUND"),
    ("GET [channel]/ref", json!({"kind": "team", "slug": "sales"}), "NOT_FOUND"),
    ("POST [@carbon]/sync/start", Value::Null, "UNREACHABLE"),
  ];
  for (key, input, code) in cases {
    assert_eq!(alice_on(key, input), code, "{key}");
  }
  let system = |key: &str, input: Value| call_now(&carbon, carbon.system(), &actor(&identity), key, input).unwrap_err().code;
  assert_eq!(system("GET [@carbon]/status", json!({})), "BAD_REQUEST");
  assert_eq!(system("GET [@carbon]/nothing", Value::Null), "NOT_FOUND");
  assert_eq!(system("GET [channel]/refs", Value::Null), "NOT_FOUND");
  assert_eq!(on(&carbon, &identity.personal, &identity, "GET [channel]/members", Value::Null).unwrap(), json!([{"identity": identity.id, "access": "manage"}]));
  assert_eq!(on(&carbon, &identity.personal, &identity, "GET [channel]/invitations", Value::Null).unwrap(), json!([]));
}

#[test]
fn a_ref_to_the_same_channel_replaces_the_last() {
  let (_directory, carbon) = carbon();
  let identity = alice(&carbon);
  for name in ["Sales", "Sales West"] {
    on(&carbon, &identity.personal, &identity, "POST [channel]/ref", json!({"channel": "c1", "kind": "team", "name": name})).unwrap();
  }
  let refs = on(&carbon, &identity.personal, &identity, "GET [channel]/refs", Value::Null).unwrap();
  assert_eq!(refs, json!([{"channel": "c1", "kind": "team", "name": "Sales West", "slug": "sales-west", "cascade": []}]));
}

#[test]
fn deletes_this_devices_copy_of_a_service_or_of_a_channel_and_its_services() {
  let (directory, carbon) = carbon();
  crate::testing::register_script(&carbon, "jots");
  let identity = alice(&carbon);
  let room = room_channel(&carbon, &identity);
  on(&carbon, &room, &identity, "POST [channel]/services", json!({"name": "jots"})).unwrap();
  let (notes, jots) = (service(&carbon, &room, "notes").id.clone(), service(&carbon, &room, "jots").id.clone());
  on(&carbon, &room, &identity, "DELETE [@carbon]/data", json!({"service": "jots"})).unwrap();
  assert!(!directory.path().join("data").join(format!("{jots}.db")).exists());
  assert!(directory.path().join("data").join(format!("{notes}.db")).exists());
  on(&carbon, &room, &identity, "DELETE [@carbon]/data", Value::Null).unwrap();
  for id in [&room, &notes] {
    assert!(!directory.path().join("data").join(format!("{id}.db")).exists(), "{id}");
    assert_eq!(space_now(&carbon, id).err().map(|error| error.status), Some(404), "{id}");
  }
}
