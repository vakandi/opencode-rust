//! Identifier generation matching OpenCode's own wire format.
//!
//! The upstream gateway validates these ids as if they came from OpenCode, so the
//! shape is part of the contract, not an internal detail. OpenCode emits a 26-character
//! suffix — a lowercase hex head then base62 — as in `msg_12b66c842001U7f8f0PM5Ac7RN`.
//! A 24-character suffix or a hyphenated 36-character UUID is refused with the same
//! FreeTierError as no fingerprint at all.

use rand::Rng;

const ALPHABET: &[u8; 62] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
const HEX: &[u8; 16] = b"0123456789abcdef";

/// The 26-character suffix OpenCode produces: 12 lowercase hex then 14 base62.
pub fn suffix() -> String {
    let mut rng = rand::thread_rng();
    let hex: String = (0..12)
        .map(|_| HEX[rng.gen_range(0..HEX.len())] as char)
        .collect();
    let tail: String = (0..14)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect();
    format!("{hex}{tail}")
}

pub fn message_id() -> String {
    format!("msg_{}", suffix())
}

pub fn part_id() -> String {
    format!("prt_{}", suffix())
}

pub fn session_id() -> String {
    format!("ses_{}", suffix())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_use_the_opencode_shape() {
        for make in [message_id, part_id, session_id] {
            let id = make();
            let (prefix, body) = id.split_once('_').expect("prefix");
            assert!(["msg", "prt", "ses"].contains(&prefix));
            assert_eq!(body.len(), 26, "{id} must carry 26 characters");
            assert!(body.chars().all(|c| c.is_ascii_alphanumeric()), "{id}");
        }
    }

    #[test]
    fn suffix_shape_matches_the_capture() {
        let body = suffix();
        assert!(body[..12].chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert!(body[12..].chars().all(|c| ALPHABET.contains(&(c as u8))));
    }

    #[test]
    fn ids_are_not_repeated() {
        let ids: std::collections::HashSet<String> = (0..200).map(|_| message_id()).collect();
        assert!(ids.len() > 190, "identifiers must not collide: {} unique of 200", ids.len());
    }
}