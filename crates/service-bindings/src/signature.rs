//! Writes a service accepts only on `[@carbon]`'s word, such as naming a space
//! the core created or listing a device it enrolled. Such a write carries a
//! signature by its op's device key over what it records: only the core holds
//! device keys, so only `[@carbon]` can make one, and the service checks it
//! against the op's device, which every replica does alike. It is an agreement
//! between `[@carbon]` and the service, not a rule of the core's.

use ed25519_dalek::{Signature, VerifyingKey};
use serde_json::{Value, json};

use crate::error::{Result, ServiceError, forbidden};
use crate::hash::{canonical, from_hex};
use crate::op::Op;

// What is signed for a write: its key and what it records, canonically encoded.
pub fn claim(key: &str, recorded: &Value) -> Vec<u8> {
  canonical(&json!({"key": key, "recorded": recorded})).into_bytes()
}

// Check that what a write records was signed by its op's device.
pub fn check(op: Option<&Op>, key: &str, recorded: &Value, signature: &str) -> Result<()> {
  let device = op.and_then(|op| op.device.as_deref()).ok_or_else(|| unsigned(key))?;
  let key_bytes: [u8; 32] = from_hex(device).and_then(|bytes| bytes.try_into().ok()).ok_or_else(|| unsigned(key))?;
  let signature_bytes: [u8; 64] = from_hex(signature).and_then(|bytes| bytes.try_into().ok()).ok_or_else(|| unsigned(key))?;
  let verifying = VerifyingKey::from_bytes(&key_bytes).map_err(|_| unsigned(key))?;
  verifying.verify_strict(&claim(key, recorded), &Signature::from_bytes(&signature_bytes)).map_err(|_| unsigned(key))
}

fn unsigned(key: &str) -> ServiceError {
  forbidden(format!("{key} records only what [@carbon] signed with the op's device key"))
}

#[cfg(test)]
mod tests {
  use super::*;
  use ed25519_dalek::{Signer, SigningKey};

  fn op(device: &str) -> Op {
    serde_json::from_value(json!({"key": "POST [channel]/services", "input": {}, "author": "a", "device": device, "version": "v1", "time": 1, "hash": "h"})).unwrap()
  }

  #[test]
  fn accepts_only_what_the_ops_device_signed() {
    let (device, other) = (SigningKey::from_bytes(&[7; 32]), SigningKey::from_bytes(&[8; 32]));
    let recorded = json!({"name": "chat", "space": "s1"});
    let signed = crate::hash::to_hex(&device.sign(&claim("POST [channel]/services", &recorded)).to_bytes());
    let device_hex = crate::hash::to_hex(device.verifying_key().as_bytes());
    let cases = [
      ("signed by the op's device", op(&device_hex), recorded.clone(), signed.clone(), Ok(())),
      ("signed by another device", op(&crate::hash::to_hex(other.verifying_key().as_bytes())), recorded.clone(), signed.clone(), Err(403)),
      ("something else recorded", op(&device_hex), json!({"name": "chat", "space": "s2"}), signed.clone(), Err(403)),
      ("no signature", op(&device_hex), recorded.clone(), String::new(), Err(403)),
    ];
    for (why, op, recorded, signature, expected) in cases {
      assert_eq!(check(Some(&op), "POST [channel]/services", &recorded, &signature).map_err(|error| error.status), expected, "{why}");
    }
  }
}
