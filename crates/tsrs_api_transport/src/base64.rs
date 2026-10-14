// Standard (RFC 4648, padded) base64, matching Go's encoding/json marshaling of []byte.

use ::base64::Engine as _;

pub fn encode(data: &[u8]) -> String {
    ::base64::engine::general_purpose::STANDARD.encode(data)
}

#[cfg(test)]
mod tests {
    #[test]
    fn rfc4648_vectors() {
        let cases = [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")];
        for (input, expected) in cases {
            assert_eq!(super::encode(input.as_bytes()), expected);
        }
    }
}
