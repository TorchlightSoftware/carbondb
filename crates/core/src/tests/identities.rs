use super::*;

#[test]
fn founds_its_own_channel_sealed_with_identity_and_a_personal_channel_by_ops() {
  let (_directory, carbon) = carbon();
  let identity = alice(&carbon);
  assert_eq!(carbon.identity("alice").unwrap(), identity);
  assert_eq!(logged(carbon.system()), vec!["POST [@carbon]/identity"]);
  let founding = &oplog::entries(carbon.system().store()).unwrap()[0];
  let effects: Vec<String> = oplog::effects(carbon.system().store(), &founding.hash).unwrap().unwrap().into_iter().map(|made| format!("{} {}", made.name, made.input["key"].as_str().unwrap_or(""))).collect();
  let expected = [
    "create ",
    "call POST [channel]/origin",
    "call POST [channel]/services",
    "call POST [identity]/origin",
    "call POST [channel]/channel",
    "call POST [channel]/services",
    "call POST [channel]/seal",
  ];
  assert_eq!(effects, expected, "its own op records every effect it made");

  let own = space_now(&carbon, &identity.channel).unwrap();
  assert_eq!(logged(&own), vec!["POST [channel]/origin", "POST [channel]/services", "POST [channel]/channel", "POST [channel]/seal"]);
  assert!(oplog::entries(own.store()).unwrap().iter().all(|op| op.author == Some(identity.id.clone()) && op.device == Some(identity.device.clone())));
  let answer = |channel: &str, key: &str, input: Value| on(&carbon, channel, &identity, key, input).unwrap();
  assert_eq!(answer(&identity.channel, "GET [channel]/origin", Value::Null), json!({"kind": "identity", "name": "alice", "manage": [identity.id]}));
  assert_eq!(answer(&identity.channel, "GET [identity]/origin", Value::Null), json!({"id": identity.id, "device": identity.device}));
  assert_eq!(answer(&identity.channel, "GET [identity]/devices", Value::Null), json!([{"pubkey": identity.device, "name": "Pixel9", "tags": ["manage"]}]));
  assert_eq!(answer(&identity.channel, "GET [channel]/ref", json!({"kind": "personal", "slug": "alice"}))["channel"], json!(identity.personal));
  let sealed = on(&carbon, &identity.channel, &identity, "POST [channel]/ref", json!({"channel": "c1", "kind": "team", "name": "Sales"})).unwrap_err();
  assert_eq!((sealed.status, sealed.message.as_str()), (403, "This channel is sealed and takes no further changes"));

  assert_eq!(answer(&identity.personal, "GET [channel]/origin", Value::Null), json!({"kind": "personal", "name": "alice", "manage": [identity.id]}));
  let services = answer(&identity.personal, "GET [channel]/services", Value::Null);
  assert_eq!((services[0]["name"].clone(), services[0]["version"].clone()), (json!("notes"), json!("notes-v1")));
  assert_eq!(answer(&identity.personal, "GET [notes]/texts", Value::Null), json!([]));
}

#[test]
fn renames_a_device_as_the_identity_itself_and_adding_one_needs_peers() {
  let (_directory, carbon) = carbon();
  let identity = alice(&carbon);
  let renamed = on(&carbon, &identity.channel, &identity, &format!("PUT [identity]/devices/{}", identity.device), json!({"name": "Phone", "tags": []})).unwrap();
  assert_eq!(renamed, json!({"pubkey": identity.device, "name": "Phone", "tags": []}));
  let stranger = Actor { identity: Some("sally".into()), device: None };
  let devices = service(&carbon, &identity.channel, "identity");
  let refused = call_now(&carbon, &devices, &stranger, &format!("PUT [identity]/devices/{}", identity.device), json!({"name": "Mine"})).unwrap_err();
  assert_eq!(refused.status, 403);
  let enrolling = on(&carbon, &identity.channel, &identity, "POST [identity]/devices", json!({"name": "Laptop"})).unwrap_err();
  assert_eq!((enrolling.status, enrolling.code.as_str()), (503, "UNREACHABLE"));
}

#[test]
fn only_the_system_database_keeps_the_secret() {
  let (directory, carbon) = carbon();
  let identity = alice(&carbon);
  let row = carbon.system().store().reader().unwrap().get("SELECT secret FROM carbon_system_identity", &[]).unwrap();
  let secret = row["secret"].as_str().unwrap().to_string();
  assert_eq!(crate::keys::credentials(&secret).unwrap().verifying_key().to_hex(), identity.device);
  // The secret's parts, as p2panda keeps them: none of them appears in any space's file.
  let parts = secret_parts(&serde_json::from_str(&secret).unwrap());
  assert!(!parts.is_empty());
  for entry in std::fs::read_dir(directory.path().join("data")).unwrap() {
    let path = entry.unwrap().path();
    if path.extension().is_some_and(|extension| extension == "db") && path.file_name().unwrap() != "system.db" {
      let kept = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).to_string();
      assert!(parts.iter().all(|part| !kept.contains(part)), "{}", path.display());
    }
  }
  let listed = call_now(&carbon, carbon.system(), &Actor::default(), "GET [@carbon]/identities", Value::Null).unwrap().value;
  assert_eq!(listed[0]["personal"], json!(identity.personal));
  assert!(listed[0].get("secret").is_none());
  let op = &oplog::entries(carbon.system().store()).unwrap()[0];
  let kept = serde_json::to_string(&(op, results::of(carbon.system().store(), &op.hash).unwrap(), oplog::effects(carbon.system().store(), &op.hash).unwrap())).unwrap();
  assert!(!kept.contains(&secret));
}

// The text a secret keeps: each string in it, and each list of bytes as text.
fn secret_parts(secret: &Value) -> Vec<String> {
  match secret {
    Value::String(text) => vec![text.clone()],
    Value::Array(items) if items.iter().all(Value::is_u64) => vec![items.iter().map(|byte| format!("{byte},")).collect()],
    Value::Array(items) => items.iter().flat_map(secret_parts).collect(),
    Value::Object(fields) => fields.values().flat_map(secret_parts).collect(),
    _ => vec![],
  }
}

#[test]
fn refuses_a_taken_or_malformed_slug_and_unregistered_services() {
  let (_directory, carbon) = carbon();
  alice(&carbon);
  let cases = [
    (json!({"slug": "alice", "device": "x"}), 409),
    (json!({"slug": "Alice", "device": "x"}), 400),
    (json!({"slug": "", "device": "x"}), 400),
    (json!({"slug": "sally", "device": "x", "services": ["crm"]}), 400),
    (json!({"slug": "alice-2", "device": "x", "code": "scanned"}), 501),
  ];
  for (input, status) in cases {
    let error = call_now(&carbon, carbon.system(), &Actor::default(), "POST [@carbon]/identity", input.clone()).unwrap_err();
    assert_eq!(error.status, status, "{input}");
  }
  assert_eq!(outcomes(carbon.system()), vec![Ok(()), Err(409), Err(400), Err(400), Err(400), Err(501)]);
}
