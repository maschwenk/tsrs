use std::sync::LazyLock;

use rustc_hash::FxHashMap;
use tsrs_core::collections::{OrderedMap, OrderedMapExt};

use crate::commandlineoption::CommandLineOption;
use crate::declsbuild::BuildOpts;
use crate::declscompiler::OptionsDeclarations;
use crate::declswatch::OptionsForWatch;

pub static CompilerNameMap: LazyLock<NameMap> = LazyLock::new(|| get_name_map_from_list(&OptionsDeclarations));
pub static BuildNameMap: LazyLock<NameMap> = LazyLock::new(|| get_name_map_from_list(&BuildOpts));
pub static WatchNameMap: LazyLock<NameMap> = LazyLock::new(|| get_name_map_from_list(&OptionsForWatch));

pub fn get_name_map_from_list(opt_decls: &[&'static CommandLineOption]) -> NameMap {
    let mut options_names = OrderedMap::with_capacity_and_hasher(opt_decls.len(), Default::default());
    let mut short_option_names = FxHashMap::default();
    for option in opt_decls {
        options_names.set(option.name.to_lowercase(), *option);
        if !option.short_name.is_empty() {
            short_option_names.insert(option.short_name.to_string(), option.name.to_string());
        }
    }
    NameMap { options_names, short_option_names }
}

pub struct NameMap {
    options_names: OrderedMap<String, &'static CommandLineOption>,
    short_option_names: FxHashMap<String, String>,
}

impl NameMap {
    pub fn get(&self, name: &str) -> Option<&'static CommandLineOption> {
        self.options_names.get(&name.to_lowercase()).copied()
    }

    pub fn get_from_short(&self, short_name: &str) -> Option<&'static CommandLineOption> {
        // returns option only if shortName is a valid short option
        let name = self.short_option_names.get(short_name)?;
        self.get(name)
    }

    pub fn get_option_declaration_from_name(&self, option_name: &str, allow_short: bool) -> Option<&'static CommandLineOption> {
        let mut option_name = option_name.to_lowercase();
        // Try to translate short option names to their full equivalents.
        if allow_short {
            if let Some(short) = self.short_option_names.get(&option_name) {
                if !short.is_empty() {
                    option_name = short.clone();
                }
            }
        }
        self.get(&option_name)
    }
}
