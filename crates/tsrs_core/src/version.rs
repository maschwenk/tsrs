use std::sync::LazyLock;

const VERSION: &str = "7.1.0-dev";

pub fn version() -> &'static str {
    VERSION
}

static VERSION_MAJOR_MINOR: LazyLock<&'static str> = LazyLock::new(|| {
    let mut seen_major = false;
    let i = VERSION.char_indices().find(|&(_, r)| {
        if r == '.' {
            if seen_major {
                return true;
            }
            seen_major = true;
        }
        false
    });
    match i {
        None => panic!("invalid version string: {VERSION}"),
        Some((i, _)) => &VERSION[..i],
    }
});

pub fn version_major_minor() -> &'static str {
    &VERSION_MAJOR_MINOR
}
