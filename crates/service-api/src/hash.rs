//! Content hashes: BLAKE3, as p2panda uses, over a canonical encoding of JSON,
//! so the same value hashes the same everywhere whatever order its keys came in.

use serde_json::Value;

// JSON with object keys sorted at every level and no whitespace.
pub fn canonical(value: &Value) -> String {
  match value {
    Value::Object(map) => {
      let mut keys: Vec<&String> = map.keys().collect();
      keys.sort();
      let fields: Vec<String> = keys.into_iter().map(|key| format!("{}:{}", Value::from(key.as_str()), canonical(&map[key]))).collect();
      format!("{{{}}}", fields.join(","))
    }
    Value::Array(items) => format!("[{}]", items.iter().map(canonical).collect::<Vec<_>>().join(",")),
    scalar => scalar.to_string(),
  }
}

// The hex BLAKE3 hash of a value's canonical encoding.
pub fn hash(value: &Value) -> String {
  blake3::hash(canonical(value).as_bytes()).to_hex().to_string()
}

pub fn to_hex(bytes: &[u8]) -> String {
  bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
  use super::*;
  use serde_json::json;

  #[test]
  fn key_order_does_not_change_the_hash() {
    let one: Value = serde_json::from_str(r#"{"b": 1, "a": {"y": [1, 2], "x": null}}"#).unwrap();
    let other: Value = serde_json::from_str(r#"{"a": {"x": null, "y": [1, 2]}, "b": 1}"#).unwrap();
    assert_eq!(hash(&one), hash(&other));
  }

  #[test]
  fn canonical_encoding_sorts_keys_and_drops_whitespace() {
    let cases = [
      (json!({"b": "two", "a": 1}), r#"{"a":1,"b":"two"}"#),
      (json!([{"z": true, "y": null}]), r#"[{"y":null,"z":true}]"#),
      (json!("quote\"d"), r#""quote\"d""#),
    ];
    for (value, expected) in cases {
      assert_eq!(canonical(&value), expected);
    }
  }

  #[test]
  fn different_values_hash_differently() {
    assert_ne!(hash(&json!({"a": 1})), hash(&json!({"a": 2})));
    assert_eq!(hash(&json!({"a": 1})).len(), 64);
  }
}
