//! A service's package: one WebAssembly component that carries its
//! `service.toml` in a custom section, `carbondb:service.toml`, so the hash
//! of the component's bytes, its version, covers its manifest too. Packaging
//! appends the section to a component a toolchain built; the core reads it
//! back when the package is registered.
//!
//! A component, like a module, is a preamble followed by sections, each an id
//! byte, its size as unsigned LEB128, and its contents; a custom section's id
//! is 0, and its contents start with its name, length-prefixed.

// The custom section a package's manifest travels in.
pub const MANIFEST_SECTION: &str = "carbondb:service.toml";

// The magic and version of a component's preamble, then its layer: 0x0d 0x00 0x01 0x00.
const PREAMBLE: usize = 8;

// A component with its manifest appended, ready to register.
pub fn embed(component: &[u8], manifest: &str) -> Result<Vec<u8>, String> {
  if manifest_of(component)?.is_some() {
    return Err("This component already carries a service.toml".into());
  }
  let mut contents = leb128(MANIFEST_SECTION.len());
  contents.extend_from_slice(MANIFEST_SECTION.as_bytes());
  contents.extend_from_slice(manifest.as_bytes());
  let mut packaged = component.to_vec();
  packaged.push(0);
  packaged.extend(leb128(contents.len()));
  packaged.extend(contents);
  Ok(packaged)
}

// The manifest a package carries, if any; an error when the bytes are not a component.
pub fn manifest_of(component: &[u8]) -> Result<Option<String>, String> {
  if component.len() < PREAMBLE || &component[..4] != b"\0asm" || component[6..8] != [1, 0] {
    return Err("Not a WebAssembly component".into());
  }
  let mut at = PREAMBLE;
  while at < component.len() {
    let id = component[at];
    let (size, read) = read_leb128(&component[at + 1..])?;
    let start = at + 1 + read;
    let contents = component.get(start..start + size).ok_or("A section runs past the end of the component")?;
    if id == 0 && let Some(manifest) = named(contents, MANIFEST_SECTION)? {
      return String::from_utf8(manifest.to_vec()).map(Some).map_err(|_| "The service.toml a component carries is not UTF-8".to_string());
    }
    at = start + size;
  }
  Ok(None)
}

// A custom section's contents after its name, when it has this name.
fn named<'a>(contents: &'a [u8], name: &str) -> Result<Option<&'a [u8]>, String> {
  let (length, read) = read_leb128(contents)?;
  let found = contents.get(read..read + length).ok_or("A custom section's name runs past its end")?;
  Ok((found == name.as_bytes()).then(|| &contents[read + length..]))
}

fn leb128(mut value: usize) -> Vec<u8> {
  let mut bytes = vec![];
  loop {
    let byte = (value & 0x7f) as u8;
    value >>= 7;
    if value == 0 {
      bytes.push(byte);
      return bytes;
    }
    bytes.push(byte | 0x80);
  }
}

// An unsigned LEB128 value and how many bytes it took.
fn read_leb128(bytes: &[u8]) -> Result<(usize, usize), String> {
  let mut value = 0usize;
  for (index, byte) in bytes.iter().take(5).enumerate() {
    value |= ((byte & 0x7f) as usize) << (7 * index);
    if byte & 0x80 == 0 {
      return Ok((value, index + 1));
    }
  }
  Err("A malformed size in the component".into())
}

#[cfg(test)]
mod tests {
  use super::*;

  // The smallest component: its preamble, and a custom section of its own.
  fn component() -> Vec<u8> {
    let mut bytes = b"\0asm\x0d\x00\x01\x00".to_vec();
    bytes.extend([0, 4, 3, b'a', b'b', b'c']);
    bytes
  }

  #[test]
  fn a_packaged_component_carries_its_manifest_after_its_own_sections() {
    let manifest = "name = \"chat\"\n".repeat(20);
    let packaged = embed(&component(), &manifest).unwrap();
    assert!(packaged.starts_with(&component()));
    assert_eq!(manifest_of(&packaged).unwrap(), Some(manifest.clone()));
    assert_eq!(manifest_of(&component()).unwrap(), None);
    assert!(embed(&packaged, &manifest).is_err(), "a component carries one manifest");
  }

  #[test]
  fn refuses_what_is_not_a_component() {
    let module = b"\0asm\x01\x00\x00\x00".to_vec();
    let cases = [("too short", b"\0asm".to_vec()), ("a core module", module), ("a section past the end", [component(), vec![0, 9, 1]].concat())];
    for (why, bytes) in cases {
      assert!(manifest_of(&bytes).is_err(), "{why}");
    }
  }
}
