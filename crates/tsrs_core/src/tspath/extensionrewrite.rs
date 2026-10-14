// Port of tspath/extensionrewrite.go. String paths retain the rooted/module-specifier distinction at callers.
use super::*;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtensionRewrite { pub source: String, pub target: String }

pub fn get_extension_rewrite<'a>(path: &str, rewrites: &'a [ExtensionRewrite], case_sensitive: bool) -> Option<&'a ExtensionRewrite> {
    let mut best: Option<&ExtensionRewrite> = None;
    for rewrite in rewrites {
        if best.is_some_and(|best| rewrite.source.len() <= best.source.len()) || path.len() <= rewrite.source.len() { continue; }
        let Some(suffix) = path.get(path.len() - rewrite.source.len()..) else { continue; };
        if if case_sensitive { suffix == rewrite.source } else { crate::stringutil::equal_fold(suffix, &rewrite.source) } { best = Some(rewrite); }
    }
    best
}

impl ExtensionRewrite {
    pub fn is_identity(&self, case_sensitive: bool) -> bool {
        if case_sensitive { self.source == self.target } else { crate::stringutil::equal_fold(&self.source, &self.target) }
    }
}

pub fn rewrite_extension(path: &str, rewrites: &[ExtensionRewrite], case_sensitive: bool) -> (String, bool) {
    let Some(rewrite) = get_extension_rewrite(path, rewrites, case_sensitive) else { return (path.to_string(), false); };
    if rewrite.is_identity(case_sensitive) { return (path.to_string(), false); }
    (path[..path.len() - rewrite.source.len()].to_string() + &rewrite.target, true)
}

pub fn should_rewrite_extension(specifier: &str, rewrites: &[ExtensionRewrite], case_sensitive: bool) -> bool {
    if !path_is_relative(specifier) || is_declaration_file_name(specifier) { return false; }
    if let Some(rewrite) = get_extension_rewrite(specifier, rewrites, case_sensitive) { return !rewrite.is_identity(case_sensitive); }
    has_ts_file_extension(specifier)
}
