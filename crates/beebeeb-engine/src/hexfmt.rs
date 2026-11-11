//! Lower-case hex for digests and 32-byte public keys: the one encoder and the one strict decoder the
//! v2 modules share (`office_v2` checks a pinned SHA-256, `share_v2` keeps sealed keys and fingerprints).

/// Lower-case hex of `bytes`, two digits per byte.
pub fn encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// 32 bytes from exactly 64 hex digits; anything else (a sign, a space, another length) is `None`.
pub fn decode32(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f";

    #[test]
    fn a_key_is_read_from_exactly_64_hex_digits_and_written_back_the_same() {
        let bytes = decode32(KEY).expect("64 hex digits");
        assert_eq!(bytes[0], 0xde);
        assert_eq!(bytes[31], 0x4f);
        assert_eq!(encode(&bytes), KEY);
        assert_eq!(decode32(&KEY.to_uppercase()), Some(bytes), "either case reads");
        let mut refused = 0;
        for bad in [
            "+a".repeat(32),
            format!("{} ", &KEY[..63]),
            KEY[..62].to_string(),
            format!("{KEY}00"),
            "zz".repeat(32),
            String::new(),
        ] {
            assert_eq!(decode32(&bad), None, "{bad}");
            refused += 1;
        }
        assert_eq!(refused, 6);
        assert_eq!(encode(&[0x00, 0x0f, 0xa0]), "000fa0");
    }
}
