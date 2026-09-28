//! Fold v1: the case- and accent-insensitive form DunneNote stores beside tag names, tag aliases,
//! metadata keys and text metadata values (`name_folded`, `alias_folded`, `key`,
//! `value_folded`).
//!
//! The steps are NFC, then Unicode lowercase (full mappings, not locale-aware), then NFD, then
//! every combining mark removed, then NFC again. Nothing is trimmed. The result depends on the
//! Unicode tables of `unicode-normalization` 0.1.24 and of Rust 1.95's `str::to_lowercase`, which
//! is why both are pinned: a writer with other tables may fold some rare characters differently
//! from DunneNote, and two tags could then collide or fail to.

use unicode_normalization::char::is_combining_mark;
use unicode_normalization::UnicodeNormalization;

/// The fold v1 form of `input`.
pub fn fold(input: &str) -> String {
    let nfc: String = input.nfc().collect();
    let lowered = nfc.to_lowercase();
    let stripped: String = lowered.nfd().filter(|c| !is_combining_mark(*c)).collect();
    stripped.nfc().collect()
}

#[cfg(test)]
mod tests {
    use super::fold;

    /// Expected outputs are DunneNote's (its own corpus, byte for byte).
    const CORPUS: &[(&str, &str)] = &[
        ("Texas", "texas"),
        ("TEXAS", "texas"),
        ("tx", "tx"),
        ("Project/Q4 Report", "project/q4 report"),
        ("Téxas", "texas"),
        ("São Paulo", "sao paulo"),
        ("Café", "cafe"),
        ("ÅNGSTRÖM", "angstrom"),
        ("naïve", "naive"),
        ("Straße", "straße"),
        ("STRASSE", "strasse"),
        ("\u{130}stanbul", "istanbul"),
        ("Cafe\u{301}", "cafe"),
        ("\u{301}\u{308}", ""),
        ("東京タワー", "東京タワー"),
        ("Tokyo東京", "tokyo東京"),
        ("Ελλάδα", "ελλαδα"),
        ("Москва́", "москва"),
        ("  Texas  ", "  texas  "),
        ("", ""),
    ];

    #[test]
    fn corpus_is_byte_exact() {
        for (input, expected) in CORPUS {
            assert_eq!(fold(input), *expected, "fold({input:?})");
        }
    }

    #[test]
    fn fold_is_idempotent() {
        for (input, _) in CORPUS {
            let once = fold(input);
            assert_eq!(fold(&once), once, "fold(fold({input:?}))");
        }
    }
}
