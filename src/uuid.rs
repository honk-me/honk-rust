use std::time::{SystemTime, UNIX_EPOCH};

/// A new UUIDv7 (RFC 9562: 48-bit Unix milliseconds, then random bits), the default
/// `Idempotency-Key`. Store it with your job if you want to retry the same event across process
/// restarts.
///
/// ```
/// let key = honk_me::new_idempotency_key();
/// assert_eq!(key.len(), 36);
/// assert_eq!(&key[14..15], "7");
/// ```
pub fn new_idempotency_key() -> String {
    uuidv7(SystemTime::now())
}

pub(crate) fn uuidv7(now: SystemTime) -> String {
    let mut b = [0u8; 16];
    getrandom::fill(&mut b).expect("honk: the operating system's random number generator failed");
    let ms = now
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    b[..6].copy_from_slice(&ms.to_be_bytes()[2..]);
    b[6] = (b[6] & 0x0f) | 0x70; // version 7
    b[8] = (b[8] & 0x3f) | 0x80; // variant 10
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// A uniformly random `u64` (full jitter for the backoff).
pub(crate) fn random_u64() -> u64 {
    let mut b = [0u8; 8];
    getrandom::fill(&mut b).expect("honk: the operating system's random number generator failed");
    u64::from_le_bytes(b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn version_variant_and_time_order() {
        let a = uuidv7(UNIX_EPOCH + Duration::from_millis(1_791_116_405_123));
        let b = uuidv7(UNIX_EPOCH + Duration::from_millis(1_791_116_405_124));
        assert_eq!(a.len(), 36);
        assert_eq!([&a[8..9], &a[13..14], &a[18..19], &a[23..24]], ["-"; 4]);
        assert_eq!(&a[14..15], "7");
        assert!(matches!(&a[19..20], "8" | "9" | "a" | "b"));
        assert!(a < b, "{a} < {b}");
        assert_eq!(
            u64::from_str_radix(&a[0..8], 16).unwrap() << 16
                | u64::from_str_radix(&a[9..13], 16).unwrap(),
            1_791_116_405_123
        );
    }
}
