use std::collections::BTreeSet;
use std::fmt::Write as _;

use miette::Diagnostic;
use thiserror::Error;

/// The reverse-hex alphabet jj uses for change ids, mapping each hex digit
/// (`0`-`9`, `a`-`f`) to a letter.
const ALPHABET: &[u8; 16] = b"zyxwvutsrqponmlk";

/// The number of random bytes in a node id. Four bytes give eight characters
/// and a ~0.01% collision chance at 1,000 nodes.
const BYTES: usize = 4;

/// A node id: eight characters from `ALPHABET`, encoding four bytes.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct NodeId([u8; BYTES]);

impl NodeId {
    pub fn generate() -> NodeId {
        let mut bytes = [0u8; BYTES];
        // An OS RNG that fails has no fallback; ids cannot be minted.
        #[allow(clippy::expect_used)]
        getrandom::fill(&mut bytes).expect("OS RNG is unavailable");
        NodeId(bytes)
    }

    pub fn from_bytes(bytes: [u8; BYTES]) -> NodeId {
        NodeId(bytes)
    }

    /// The id's eight characters.
    fn encoded(&self) -> [u8; BYTES * 2] {
        let nibbles = self.0.iter().flat_map(|b| [b >> 4, b & 0x0f]);
        let mut out = [0u8; BYTES * 2];
        for (slot, nibble) in out.iter_mut().zip(nibbles) {
            // Nibbles are masked to four bits, so the index is within ALPHABET.
            #[allow(clippy::indexing_slicing)]
            {
                *slot = ALPHABET[nibble as usize];
            }
        }
        out
    }

    /// Whether this id starts with `prefix`.
    fn starts_with(&self, prefix: &str) -> bool {
        let encoded = self.encoded();
        encoded.starts_with(prefix.as_bytes())
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.encoded() {
            f.write_char(byte as char)?;
        }
        Ok(())
    }
}

impl std::str::FromStr for NodeId {
    type Err = InvalidNodeId;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.len() != BYTES * 2 {
            return Err(InvalidNodeId);
        }
        let mut bytes = [0u8; BYTES];
        for (chunk, byte) in s.as_bytes().chunks(2).zip(bytes.iter_mut()) {
            let hi = from_alphabet(chunk.first().copied())?;
            let lo = from_alphabet(chunk.get(1).copied())?;
            *byte = hi
                .checked_mul(16)
                .and_then(|v| v.checked_add(lo))
                .ok_or(InvalidNodeId)?;
        }
        Ok(NodeId(bytes))
    }
}

fn from_alphabet(c: Option<u8>) -> Result<u8, InvalidNodeId> {
    let Some(c) = c else {
        return Err(InvalidNodeId);
    };
    let nibble = ALPHABET.iter().position(|a| *a == c).ok_or(InvalidNodeId)?;
    u8::try_from(nibble).map_err(|_| InvalidNodeId)
}

#[derive(Error, Diagnostic, Debug)]
#[error("invalid node id: expected eight characters from the alphabet k-z")]
#[diagnostic(code(viri_core::id::invalid))]
pub struct InvalidNodeId;

/// The shortest prefix of `id` that matches no other id in `ids`.
///
/// `ids` must contain `id`. The returned prefix has at least one character;
/// eight only when `ids` also holds an id sharing all but its last character.
pub fn shortest_prefix(id: &NodeId, ids: &BTreeSet<NodeId>) -> String {
    let s = id.to_string();
    for len in 1..s.len() {
        let unique = ids
            .iter()
            .all(|other| other == id || !other.starts_with(&s[..len]));
        if unique {
            return s[..len].to_owned();
        }
    }
    s
}

#[derive(Error, Diagnostic, Debug, PartialEq)]
pub enum PrefixError {
    #[error("no node id starts with `{0}`")]
    #[diagnostic(code(viri_core::id::unresolved))]
    NotFound(String),
    #[error("`{prefix}` matches several node ids: {}", matches.iter().map(ToString::to_string).collect::<Vec<_>>().join(", "))]
    #[diagnostic(code(viri_core::id::ambiguous), help("use a longer prefix"))]
    Ambiguous {
        prefix: String,
        matches: Vec<NodeId>,
    },
}

/// Resolves a prefix to the unique node id it matches in `ids`.
pub fn resolve_prefix(prefix: &str, ids: &BTreeSet<NodeId>) -> Result<NodeId, PrefixError> {
    let matches: Vec<NodeId> = ids
        .iter()
        .copied()
        .filter(|id| id.starts_with(prefix))
        .collect();
    match matches.as_slice() {
        [] => Err(PrefixError::NotFound(prefix.to_owned())),
        [id] => Ok(*id),
        _ => Err(PrefixError::Ambiguous {
            prefix: prefix.to_owned(),
            matches,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> NodeId {
        s.parse().unwrap()
    }

    #[test]
    fn generates_valid_ids() {
        for _ in 0..100 {
            let id = NodeId::generate();
            let roundtrip: NodeId = id.to_string().parse().unwrap();
            assert_eq!(id, roundtrip);
        }
    }

    #[test]
    fn rejects_malformed_ids() {
        for bad in ["", "z", "zzzzzzzzz", "zzzzzzza", "abcdefgh"] {
            assert!(bad.parse::<NodeId>().is_err(), "{bad} should be invalid");
        }
    }

    #[test]
    fn alphabet_maps_hex_in_reverse() {
        assert_eq!(id("zzzzzzzk").0, [0x00, 0x00, 0x00, 0x0f]);
        assert_eq!(id("zkzkzkzk").0, [0x0f, 0x0f, 0x0f, 0x0f]);
        assert_eq!(id("zkzkzkzk").to_string(), "zkzkzkzk");
    }

    #[test]
    fn resolves_unique_prefixes() {
        let ids = BTreeSet::from([id("zkqpmzxy"), id("zkqpmlmn"), id("tsrqponm")]);
        assert_eq!(resolve_prefix("tsr", &ids).unwrap(), id("tsrqponm"));
        assert_eq!(resolve_prefix("zkqpmzx", &ids).unwrap(), id("zkqpmzxy"));
        assert_eq!(
            resolve_prefix("zkqpm", &ids).unwrap_err(),
            PrefixError::Ambiguous {
                prefix: "zkqpm".to_owned(),
                matches: vec![id("zkqpmzxy"), id("zkqpmlmn")],
            }
        );
        assert_eq!(
            resolve_prefix("w", &ids).unwrap_err(),
            PrefixError::NotFound("w".to_owned())
        );
    }

    #[test]
    fn shortest_prefix_needs_only_as_many_chars_as_necessary() {
        let ids = BTreeSet::from([id("zkqpmzxy"), id("tsrqponm")]);
        assert_eq!(shortest_prefix(&id("zkqpmzxy"), &ids), "z");
        let ids = BTreeSet::from([id("zkqpmzxy"), id("zkqpmzxx")]);
        assert_eq!(shortest_prefix(&id("zkqpmzxy"), &ids), "zkqpmzxy");
    }
}
