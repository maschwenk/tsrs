use tsrs_core::TextPos;

// source.go:5
pub trait Source {
    fn text(&self) -> &str;
    fn file_name(&self) -> &str;
    fn ecma_line_map(&self) -> &[TextPos];
}
