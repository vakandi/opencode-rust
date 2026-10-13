//! Identifier generation matching OpenCode's own wire format.
//!
//! The upstream gateway validates these ids as if they came from OpenCode, so the bytes
//! are part of the contract, not an internal detail. OpenCode emits a 26-character suffix
//! whose first 12 characters are a *timestamp*, not noise — see
//! `packages/schema/src/identifier.ts` upstream:
//!
//! ```text
//! current = BigInt(timestamp) * 0x1000n + BigInt(counter)   // timestamp = Date.now()
//! value   = descending ? ~current : current
//! time    = low 48 bits of value, as 6 big-endian hex bytes
//! tail    = 14 random bytes, each mapped through a base62 alphabet
//! ```
//!
//! Random ids of the correct *length* are refused with FreeTierError exactly like no
//! fingerprint at all, so the timestamp half must be reproduced, not just its width.
//! Sessions are descending, messages and parts ascending.

use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rand::Rng;

const ALPHABET: &[u8; 62] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

/// Last timestamp seen and how many ids were minted within it. Upstream keeps this in
/// module scope; ids minted in the same millisecond must still differ.
static CLOCK: Mutex<(u64, u64)> = Mutex::new((0, 0));

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn next_counter(timestamp: u64) -> u64 {
    let mut clock = CLOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if clock.0 != timestamp {
        clock.0 = timestamp;
        clock.1 = 0;
    }
    clock.1 += 1;
    clock.1
}

fn suffix(descending: bool) -> String {
    let timestamp = now_millis();
    let current = (timestamp as u128) * 0x1000 + next_counter(timestamp) as u128;
    let value = if descending { !current } else { current };
    let time: String = (0..6)
        .map(|index| format!("{:02x}", ((value >> (40 - 8 * index)) & 0xff) as u8))
        .collect();

    let mut rng = rand::thread_rng();
    let tail: String = (0..14)
        .map(|_| ALPHABET[(rng.gen::<u8>() % ALPHABET.len() as u8) as usize] as char)
        .collect();

    format!("{time}{tail}")
}

pub fn message_id() -> String {
    format!("msg_{}", suffix(false))
}

pub fn part_id() -> String {
    format!("prt_{}", suffix(false))
}

pub fn session_id() -> String {
    format!("ses_{}", suffix(true))
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
        let body = suffix(false);
        assert!(body[..12].chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert!(body[12..].chars().all(|c| ALPHABET.contains(&(c as u8))));
    }

    #[test]
    fn ids_are_not_repeated() {
        let ids: std::collections::HashSet<String> = (0..200).map(|_| message_id()).collect();
        assert_eq!(ids.len(), 200, "identifiers must not collide");
    }

    #[test]
    fn head_encodes_the_current_clock() {
        let head = |descending: bool| -> u128 {
            let timestamp = now_millis();
            let current = (timestamp as u128) * 0x1000 + 1;
            let value = if descending { !current } else { current };
            u128::from_str_radix(
                &(0..6)
                    .map(|i| format!("{:02x}", ((value >> (40 - 8 * i)) & 0xff) as u8))
                    .collect::<String>(),
                16,
            )
            .unwrap()
        };
        assert!(head(false) > 1 << 40, "ascending head must be clock-shaped");
        assert!(head(true) > 1 << 40, "descending head must be clock-shaped");
        assert_ne!(head(false), head(true));
    }

    #[test]
    fn ids_minted_within_one_millisecond_still_differ() {
        let a = message_id();
        let b = message_id();
        assert_ne!(a, b);
        assert_eq!(&a[..12], &b[..12], "same millisecond keeps the same head");
    }
}