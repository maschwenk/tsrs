#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub enum LanguageVariant {
    #[default]
    Standard = 0,
    JSX = 1,
}

impl LanguageVariant {
    pub fn string(self) -> &'static str {
        match self {
            LanguageVariant::Standard => "LanguageVariantStandard",
            LanguageVariant::JSX => "LanguageVariantJSX",
        }
    }
}

impl std::fmt::Display for LanguageVariant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.string())
    }
}
