//! Sibling positions in the page tree (`nodes.position`).
//!
//! A position is a fractional index (the `ZenoIndex` of the `fractional_index` crate, 1.0.1)
//! followed by one sentinel byte `0x80`, written as lowercase hex. Siblings sort by plain byte
//! order of the stored string. The first child of an empty parent is `"80"`; appending after it
//! gives `"c080"`, then `"c180"`, and so on; inserting before it gives `"4080"`.

use fractional_index::ZenoIndex;

use crate::error::{Error, Result};

/// The byte appended to every encoded position.
pub const SENTINEL: u8 = 0x80;

/// Longest stored position, in characters (the schema's CHECK).
pub const MAX_LEN: usize = 256;

/// The position of the first child of an empty parent: `"80"`.
pub fn first() -> String {
    encode(&ZenoIndex::default())
}

/// A position that sorts after `existing`.
pub fn after(existing: &str) -> Result<String> {
    checked(encode(&ZenoIndex::new_after(&parse(existing)?)))
}

/// A position that sorts before `existing`.
pub fn before(existing: &str) -> Result<String> {
    checked(encode(&ZenoIndex::new_before(&parse(existing)?)))
}

/// A position strictly between `lo` and `hi` (`lo < hi`).
pub fn between(lo: &str, hi: &str) -> Result<String> {
    if lo >= hi {
        return Err(Error::Invalid(format!(
            "position {lo:?} does not sort before {hi:?}"
        )));
    }
    let mid = ZenoIndex::new_between(&parse(lo)?, &parse(hi)?)
        .ok_or_else(|| Error::Invalid(format!("no position between {lo:?} and {hi:?}")))?;
    checked(encode(&mid))
}

/// Parse a stored position. Fails unless it is hex ending in the sentinel byte.
pub fn parse(s: &str) -> Result<ZenoIndex> {
    let malformed = || Error::Malformed(format!("{s:?} is not a DunneNote position"));
    if s.is_empty() || s.len() > MAX_LEN || s.bytes().any(|b| b.is_ascii_uppercase()) {
        return Err(malformed());
    }
    let mut bytes = hex::decode(s).map_err(|_| malformed())?;
    match bytes.pop() {
        Some(SENTINEL) => Ok(ZenoIndex::from_bytes(bytes)),
        _ => Err(malformed()),
    }
}

fn encode(z: &ZenoIndex) -> String {
    let mut bytes = z.as_bytes().to_vec();
    bytes.push(SENTINEL);
    hex::encode(bytes)
}

fn checked(s: String) -> Result<String> {
    if s.len() > MAX_LEN {
        return Err(Error::Invalid(
            "the position would exceed 256 characters; too many insertions at one spot".into(),
        ));
    }
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_dunnenote_sequences() {
        assert_eq!(first(), "80");
        assert_eq!(after("80").unwrap(), "c080");
        assert_eq!(after("c080").unwrap(), "c180");
        assert_eq!(before("80").unwrap(), "4080");
        let mid = between("80", "c080").unwrap();
        assert!("80" < mid.as_str() && mid.as_str() < "c080");
    }

    #[test]
    fn appends_stay_ordered() {
        let mut p = first();
        for _ in 0..600 {
            let next = after(&p).unwrap();
            assert!(next > p);
            p = next;
        }
    }

    #[test]
    fn rejects_foreign_strings() {
        assert!(parse("").is_err());
        assert!(parse("zz").is_err());
        assert!(parse("c0").is_err()); // no sentinel
        assert!(parse("C080").is_err());
        assert!(between("c080", "80").is_err());
    }
}
