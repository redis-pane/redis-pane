//! Which Cluster slot a key lives in (M2 task 11, ADR-0022).
//!
//! Pure: a CRC16 and a hash-tag rule, no I/O. The core needs it to know, before
//! anything is sent, that `RENAME` between two slots cannot work on a Cluster
//! (it fails with `CROSSSLOT`), so the preview can refuse rather than let the
//! server do it afterwards.

/// The number of hash slots in a Redis Cluster.
pub const SLOTS: u16 = 16384;

/// CRC16/XMODEM (polynomial `0x1021`, initial value `0`), the checksum Redis
/// Cluster uses for key hashing.
fn crc16(bytes: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &byte in bytes {
        crc ^= u16::from(byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

/// The part of the key that is hashed: the body of the first `{...}`, if that
/// body is not empty, else the whole key (the hash-tag rule).
fn hashed_part(key: &[u8]) -> &[u8] {
    if let Some(open) = key.iter().position(|&b| b == b'{')
        && let Some(len) = key[open + 1..].iter().position(|&b| b == b'}')
        && len > 0
    {
        return &key[open + 1..open + 1 + len];
    }
    key
}

/// The slot a key hashes to, `0..16384`.
pub fn key_slot(key: &[u8]) -> u16 {
    crc16(hashed_part(key)) % SLOTS
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The reference vector from the Redis Cluster specification.
    #[test]
    fn crc16_matches_the_specification_vector() {
        assert_eq!(crc16(b"123456789"), 0x31C3);
        assert_eq!(key_slot(b"123456789"), 0x31C3 % SLOTS);
    }

    /// Slots read off `CLUSTER KEYSLOT` on a real server.
    #[test]
    fn known_slots() {
        assert_eq!(key_slot(b"foo"), 12182);
        assert_eq!(key_slot(b"bar"), 5061);
        assert_eq!(key_slot(b"hello"), 866);
        assert_eq!(key_slot(b""), 0);
    }

    #[test]
    fn a_hash_tag_decides_the_slot() {
        assert_eq!(key_slot(b"{x}a"), key_slot(b"{x}b"));
        assert_eq!(key_slot(b"{x}a"), key_slot(b"x"));
        assert_eq!(key_slot(b"user:{42}:cart"), key_slot(b"42"));
    }

    #[test]
    fn an_empty_tag_is_not_a_tag() {
        assert_eq!(key_slot(b"foo{}bar"), crc16(b"foo{}bar") % SLOTS);
        // Only the first `{` counts, and an unclosed one is not a tag.
        assert_eq!(key_slot(b"a{b"), crc16(b"a{b") % SLOTS);
        assert_eq!(key_slot(b"{a}{b}"), key_slot(b"a"));
        assert_eq!(key_slot(b"}{a}"), key_slot(b"a"));
    }

    #[test]
    fn different_untagged_keys_usually_differ() {
        assert_ne!(key_slot(b"a"), key_slot(b"b"));
    }
}
