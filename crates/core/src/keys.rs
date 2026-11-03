//! Device key pairs: Ed25519, generated on this machine. The secret half stays
//! in the system database and never appears in an op.

use ed25519_dalek::SigningKey;

use crate::error::{Result, internal};
use crate::hash::to_hex;

pub struct KeyPair {
  pub public: String,
  pub secret: String,
}

pub fn generate() -> Result<KeyPair> {
  let mut seed = [0u8; 32];
  getrandom::fill(&mut seed).map_err(|error| internal(format!("No randomness for a key pair: {error}")))?;
  let key = SigningKey::from_bytes(&seed);
  Ok(KeyPair { public: to_hex(key.verifying_key().as_bytes()), secret: to_hex(&seed) })
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn each_key_pair_is_new() {
    let (one, other) = (generate().unwrap(), generate().unwrap());
    assert_ne!(one.public, other.public);
    assert_eq!(one.public.len(), 64);
    assert_ne!(one.public, one.secret);
  }
}
