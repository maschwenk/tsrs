static IGNORED_PATHS: &[&str] = &["/node_modules/.", "/.git", ".#"];

pub fn contains_ignored_path(path: &str) -> bool {
    for pattern in IGNORED_PATHS {
        if path.contains(pattern) {
            return true;
        }
    }
    false
}
