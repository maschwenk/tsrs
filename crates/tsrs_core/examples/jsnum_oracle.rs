use std::io::{self, BufRead, Write};

use tsrs_core::jsnum::{Number, from_string};

fn main() {
    let mut out = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let line = line.unwrap();
        let (mode, value) = line.split_once(' ').unwrap();
        match mode {
            "format" => {
                let bits = u64::from_str_radix(value, 16).unwrap();
                writeln!(out, "{}", Number(f64::from_bits(bits)).string()).unwrap();
            }
            "parse" => writeln!(out, "{:016x}", from_string(value).0.to_bits()).unwrap(),
            _ => panic!("unknown mode: {mode}"),
        }
    }
}
