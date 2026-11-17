//! Device key pairs, generated on this machine as p2panda credentials: an
//! Ed25519 signing key, whose public half is the device's key and the p2panda
//! author of every op it makes, and the X25519 identity secret p2panda agrees
//! encryption keys with. Both halves are kept together, in p2panda's own
//! serialized form, as the device's secret in the system database; it never
//! appears in an op.

use p2panda::Credentials;

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

#[cfg(test)]
mod tests {
  use super::*;

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
}
