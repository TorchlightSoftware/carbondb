//! Space names: a kind and a lowercase kebab-case slug, as in `team:sales`. A
//! slug is a local pet name, unique among the refs of the space holding it.

use crate::error::{Result, bad_request};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpaceName {
  pub kind: String,
  pub slug: String,
}

impl SpaceName {
  pub fn parse(name: &str) -> Result<SpaceName> {
    let (kind, slug) = name.split_once(':').ok_or_else(|| bad_request(format!("'{name}' is not a space name, which is kind:slug")))?;
    if kind.is_empty() || slug.is_empty() || slugify(slug) != slug {
      return Err(bad_request(format!("'{name}' is not a space name, which is kind:slug with a lowercase kebab-case slug")));
    }
    Ok(SpaceName { kind: kind.to_string(), slug: slug.to_string() })
  }
}

// A name as a lowercase kebab-case slug: "Randall's Playhouse" becomes "randalls-playhouse".
pub fn slugify(name: &str) -> String {
  let words: Vec<String> = name
    .to_lowercase()
    .replace(['\'', '’'], "")
    .split(|character: char| !character.is_ascii_alphanumeric())
    .filter(|word| !word.is_empty())
    .map(str::to_string)
    .collect();
  words.join("-")
}

// The slug, or the slug with -2, -3 and so on when it is already taken.
pub fn unique_slug(slug: &str, taken: impl Fn(&str) -> bool) -> String {
  std::iter::once(slug.to_string())
    .chain((2..).map(|n| format!("{slug}-{n}")))
    .find(|candidate| !taken(candidate))
    .expect("an untaken slug exists")
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn slugifies_names() {
    let cases = [
      ("Randall's Playhouse", "randalls-playhouse"),
      ("FooCorp", "foocorp"),
      ("  Sales & Marketing  ", "sales-marketing"),
      ("team-sales", "team-sales"),
      ("Ünïcode Room 2", "n-code-room-2"),
    ];
    for (name, slug) in cases {
      assert_eq!(slugify(name), slug, "{name}");
    }
  }

  #[test]
  fn adds_a_number_when_a_slug_is_taken() {
    let taken = ["room", "room-2"];
    assert_eq!(unique_slug("room", |slug| taken.contains(&slug)), "room-3");
    assert_eq!(unique_slug("lobby", |slug| taken.contains(&slug)), "lobby");
  }

  #[test]
  fn parses_space_names() {
    assert_eq!(SpaceName::parse("team:sales").unwrap(), SpaceName { kind: "team".into(), slug: "sales".into() });
    for name in ["team", "team:", ":sales", "team:Sales", "team:sales room"] {
      assert_eq!(SpaceName::parse(name).unwrap_err().status, 400, "{name}");
    }
  }
}
