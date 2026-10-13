use rand::Rng;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

const BASE62_CHARS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
const LENGTH: usize = 26;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prefix {
    Session,
    Message,
    Permission,
    Question,
    User,
    Part,
    Pty,
    Tool,
}

impl Prefix {
    fn as_str(&self) -> &'static str {
        match self {
            Prefix::Session => "ses",
            Prefix::Message => "msg",
            Prefix::Permission => "per",
            Prefix::Question => "que",
            Prefix::User => "usr",
            Prefix::Part => "prt",
            Prefix::Pty => "pty",
            Prefix::Tool => "tool",
        }
    }
}

static LAST_TIMESTAMP: AtomicU64 = AtomicU64::new(0);
static COUNTER: Mutex<u32> = Mutex::new(0);

fn random_base62(length: usize) -> String {
    let mut rng = rand::thread_rng();
    let mut result = String::with_capacity(length);
    for _ in 0..length {
        let idx = rng.gen_range(0..62);
        result.push(BASE62_CHARS[idx] as char);
    }
    result
}

fn get_counter() -> u32 {
    let mut counter = COUNTER.lock().unwrap();
    *counter += 1;
    *counter
}

pub fn create(prefix: Prefix, descending: bool, timestamp: Option<u64>) -> String {
    let current_timestamp = timestamp.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    });

    let last = LAST_TIMESTAMP.load(Ordering::Relaxed);
    if current_timestamp != last {
        LAST_TIMESTAMP.store(current_timestamp, Ordering::Relaxed);
        let mut counter = COUNTER.lock().unwrap();
        *counter = 0;
    }

    let counter_val = get_counter();
    let mut now = u64::from(current_timestamp) * 0x1000 + u64::from(counter_val);

    if descending {
        now = !now;
    }

    let mut time_bytes = [0u8; 6];
    for i in 0..6 {
        time_bytes[i] = ((now >> (40 - 8 * i)) & 0xff) as u8;
    }

    let hex_time = hex::encode(time_bytes);
    let random_part = random_base62(LENGTH - 12);

    format!("{}_{}{}", prefix.as_str(), hex_time, random_part)
}

pub fn timestamp(id: &str) -> Option<u64> {
    let parts: Vec<&str> = id.split('_').collect();
    if parts.len() != 2 {
        return None;
    }

    let hex = parts[1].get(0..12)?;
    let encoded = u64::from_str_radix(hex, 16).ok()?;
    Some(encoded / 0x1000)
}

pub fn validate_prefix(id: &str, expected: Prefix) -> bool {
    id.starts_with(expected.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_id() {
        let id = create(Prefix::Session, false, None);
        assert!(id.starts_with("ses_"));
        assert_eq!(id.len(), 30);
    }

    #[test]
    fn test_timestamp_extraction() {
        let id = create(Prefix::Session, false, Some(1700000000000));
        let ts = timestamp(&id);
        assert!(ts.is_some());
    }

    #[test]
    fn test_validate_prefix() {
        let id = create(Prefix::Session, false, None);
        assert!(validate_prefix(&id, Prefix::Session));
        assert!(!validate_prefix(&id, Prefix::Message));
    }
}
