// compare.go:7
pub fn count_path_components(path: &str) -> i32 {
    let mut initial = 0;
    if path.starts_with("./") {
        initial = 2;
    }
    path[initial..].matches('/').count() as i32
}
