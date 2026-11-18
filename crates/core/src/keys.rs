//! Device key pairs, generated on this machine as p2panda credentials: an
//! Ed25519 signing key, whose public half is the device's key and the p2panda
//! author of every op it makes, and the X25519 identity secret p2panda agrees
//! encryption keys with. Both halves are kept together, in p2panda's own
//! serialized form, as the device's secret in the system database; it never
//! appears in an op. `[@carbon]` signs with it what a service records only on
//! its word (`carbondb_service_bindings::signature`).

use p2panda::Credentials;
use p2panda_core::identity::Signer;

use crate::error::{Result, internal};

pub struct KeyPair {
  pub public: String,
  pub secret: String,
}

pub fn generate() -> Result<KeyPair> {
  let credentials = Credentials::generate();
  let secret = serde_json::to_string(&credentials).map_err(|error| internal(format!("Cannot keep a device's credentials: {error}")))?;
  Ok(KeyPair { public: credentials.verifying_key().to_hex(), secret })
}

// The credentials a device's secret keeps.
pub fn credentials(secret: &str) -> Result<Credentials> {
  serde_json::from_str(secret).map_err(|error| internal(format!("A device's secret is not p2panda credentials: {error}")))
}

// A device's signature over bytes, in hex.
pub fn sign(credentials: &Credentials, bytes: &[u8]) -> String {
  credentials.sign(bytes).to_hex()
}

#[cfg(test)]
mod tests {
  use super::*;
  use carbondb_service_bindings::signature::{check, claim};
  use serde_json::json;

  #[test]
  fn each_key_pair_is_new() {
    let (one, other) = (generate().unwrap(), generate().unwrap());
    assert_ne!(one.public, other.public);
    assert_eq!(one.public.len(), 64);
    assert!(!one.secret.contains(&one.public));
  }

  #[test]
  fn a_secret_keeps_the_device_key_and_the_identity_secret() {
    let pair = generate().unwrap();
    let (kept, again) = (credentials(&pair.secret).unwrap(), credentials(&pair.secret).unwrap());
    assert_eq!(kept.verifying_key().to_hex(), pair.public);
    assert_eq!(kept.identity_key().unwrap(), again.identity_key().unwrap());
    assert_eq!(credentials("not credentials").unwrap_err().status, 500);
  }

  #[test]
  fn a_device_signs_what_a_service_checks_against_the_ops_device() {
    let pair = generate().unwrap();
    let recorded = json!({"pubkey": "b2", "name": "Laptop", "tags": []});
    let signed = sign(&credentials(&pair.secret).unwrap(), &claim("POST [identity]/devices", &recorded));
    let op = |device: &str| serde_json::from_value::<crate::Op>(json!({"key": "POST [identity]/devices", "input": {}, "author": "a", "device": device, "version": "v1", "time": 1, "hash": "h"})).unwrap();
    assert!(check(Some(&op(&pair.public)), "POST [identity]/devices", &recorded, &signed).is_ok());
    assert_eq!(check(Some(&op(&generate().unwrap().public)), "POST [identity]/devices", &recorded, &signed).unwrap_err().status, 403);
  }
}
