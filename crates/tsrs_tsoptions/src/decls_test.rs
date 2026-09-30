use rustc_hash::FxHashMap;

use crate::commandlineoption::CommandLineOption;
use crate::declscompiler::OPTIONS_DECLARATIONS;
use crate::parsinghelpers::for_each_compiler_options_field;

// Go compares the CompilerOptions struct fields (and their json tags) with the declarations; the Rust
// field list with JSON names is `for_each_compiler_options_field`.
#[test]
fn test_compiler_options_declaration() {
    let mut decls: FxHashMap<String, &'static CommandLineOption> = FxHashMap::default();
    for decl in OPTIONS_DECLARATIONS.iter() {
        decls.insert(decl.name.to_lowercase(), decl);
    }

    let internal_options = [
        "allowNonTsExtensions",
        "build",
        "configFilePath",
        "noDtsResolution",
        "noEmitForJsFiles",
        "pathsBasePath",
        "suppressOutputPathCheck",
        "build",
    ];
    let internal_options_map: FxHashMap<String, &str> = internal_options.iter().map(|o| (o.to_lowercase(), *o)).collect();

    let mut errors: Vec<String> = Vec::new();
    macro_rules! check {
        ($($field:ident: $json:literal,)*) => {
            $(
                let lower_name = $json.to_lowercase();
                match decls.remove(&lower_name) {
                    Some(decl) => {
                        if decl.name != $json {
                            errors.push(format!("Field {} has json name {}, but the option declaration has name {}", stringify!($field), $json, decl.name));
                        }
                    }
                    None => match internal_options_map.get(&lower_name) {
                        Some(name) => {
                            if *name != $json {
                                errors.push(format!("Field {} has json name {}, but the internal option has name {}", stringify!($field), $json, name));
                            }
                        }
                        None => errors.push(format!("CompilerOptions.{} has no options declaration", stringify!($field))),
                    },
                }
            )*
        };
    }
    for_each_compiler_options_field!(check);

    let skipped_options = ["plugins"];
    for opt in skipped_options {
        decls.remove(&opt.to_lowercase());
    }

    for decl in decls.values() {
        errors.push(format!("Option declaration {} is not present in CompilerOptions", decl.name));
    }
    assert!(errors.is_empty(), "{errors:#?}");
}
