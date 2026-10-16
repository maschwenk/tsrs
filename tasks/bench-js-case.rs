use std::hint::black_box;
use tsrs_core::stringutil::{encode_js_string_rune, to_lower_js, to_upper_js};

fn hash_output(hash: &mut u64, text: &str) {
    for byte in text.len().to_le_bytes().into_iter().chain(text.bytes()) {
        *hash = (*hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
    }
}

fn main() {
    let mode = std::env::args().nth(1).unwrap();
    if mode == "verify" {
        let mut hash = 0xcbf29ce484222325;
        for cp in 0..=0x10ffff {
            let input = encode_js_string_rune(cp);
            hash_output(&mut hash, &to_lower_js(&input));
            hash_output(&mut hash, &to_upper_js(&input));
        }
        let context: Vec<String> = [
            0x41, 0x61, 0x3a3, 0x3c3, 0x3c2, 0x345, 0x307, 0x20, 0x27, 0x130, 0xdf, 0xfb03,
            0x10400, 0xd800, 0xdc00, 0xaa, 0x1c89,
        ]
        .into_iter()
        .map(encode_js_string_rune)
        .collect();
        for a in &context {
            for b in &context {
                for c in &context {
                    let input = format!("{a}{b}{c}");
                    hash_output(&mut hash, &to_lower_js(&input));
                    hash_output(&mut hash, &to_upper_js(&input));
                }
            }
        }
        println!("{hash:016x}");
        return;
    }
    let (text, case): (&str, fn(&str) -> String) = match mode.as_str() {
        "ascii-lower" => (
            "THE_QUICK_BROWN_FOX_JUMPS_OVER_THE_LAZY_DOG_0123456789",
            to_lower_js,
        ),
        "ascii-upper" => (
            "the_quick_brown_fox_jumps_over_the_lazy_dog_0123456789",
            to_upper_js,
        ),
        "ascii-unchanged" => ("already_lowercase_identifiers_0123456789", to_lower_js),
        "unicode-lower" => (
            "İSPANYOL ΟΣ ΟΣΑ AΣ\u{345}A Ω ß ﬁ ﬃ ǅ 𐐀 𐐨 你好 ᾈ ᾨ ᾲ ᾷ",
            to_lower_js,
        ),
        "unicode-upper" => (
            "İspanyol ος οσα aσ\u{345}a ω ß ﬁ ﬃ ǅ 𐐀 𐐨 你好 ᾀ ᾠ ᾲ ᾷ",
            to_upper_js,
        ),
        _ => panic!("unknown mode"),
    };
    let mut total = 0;
    for _ in 0..1_000_000 {
        total += black_box(case(black_box(text))).len();
    }
    println!("{total}");
}
