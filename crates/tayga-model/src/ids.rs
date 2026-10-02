#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct TraceId(pub [u8; 16]);

impl TraceId {
    /// `None` for anything that is not 16 bytes, and for the all-zero id (invalid per W3C).
    pub fn from_slice(bytes: &[u8]) -> Option<Self> {
        let arr: [u8; 16] = bytes.try_into().ok()?;
        (arr != [0u8; 16]).then_some(Self(arr))
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

pub fn hex_id(bytes: &[u8]) -> String {
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_slice_accepts_16_nonzero_bytes() {
        let id = TraceId::from_slice(&[1u8; 16]).unwrap();
        assert_eq!(id.to_hex(), "01010101010101010101010101010101");
    }

    #[test]
    fn from_slice_rejects_wrong_length_and_zero() {
        assert!(TraceId::from_slice(&[]).is_none());
        assert!(TraceId::from_slice(&[1u8; 8]).is_none());
        assert!(TraceId::from_slice(&[0u8; 16]).is_none());
    }

    #[test]
    fn hex_id_of_empty_is_empty() {
        assert_eq!(hex_id(&[]), "");
        assert_eq!(hex_id(&[0xab, 0x01]), "ab01");
    }
}
