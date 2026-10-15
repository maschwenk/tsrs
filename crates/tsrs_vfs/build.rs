use std::{env, fs, path::PathBuf};

include!("src/bundled/libs_generated.rs");

fn main() {
    println!("cargo:rerun-if-changed=libs");
    println!("cargo:rerun-if-changed=src/bundled/libs_generated.rs");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    for name in LIB_NAMES {
        let contents = fs::read(PathBuf::from("libs").join(name)).unwrap();
        // HC only runs on the build host; the target uses the safe Rust LZ4 block decoder.
        let compressed = lz4::block::compress(&contents, Some(lz4::block::CompressionMode::HIGHCOMPRESSION(9)), false).unwrap();
        fs::write(out.join(name), compressed).unwrap();
    }
}
