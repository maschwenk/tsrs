use tsrs_ast::{Diagnostic, SourceFileParseOptions};
use tsrs_core::collections::OrderedMap;
use tsrs_core::json;
use tsrs_core::tspath::{self, ComparePathsOptions, Path};
use tsrs_core::{CompilerOptions, ModuleKind, ModuleResolutionKind, ScriptKind, ScriptTarget, P};
use tsrs_diagnostics as diagnostics;
use tsrs_vfs::{walk_dir, FS};

use crate::commandlineoption::CompilerOptionsValue;
use crate::commandlineparser::parse_command_line;
use crate::commandlineparser_test::{check_baseline, repo_root};
use crate::parsedcommandline::ParsedCommandLine;
use crate::testutil::{compiler_options_to_json, format_diagnostics_with_color_and_context, option_value_to_json, type_acquisition_to_json};
use crate::tsconfigparsing::{
    get_parsed_command_line_of_config_file, new_tsconfig_source_file_from_file_path, parse_config_file_text_to_json,
    parse_extended_config, parse_json_config_file_content, parse_json_source_file_config_file_content, ExtendedConfigCache,
    ExtendedConfigCacheEntry, ParseConfigHost, TsConfigSourceFile,
};
use crate::tsoptionstest::{get_parsed_command_line, new_vfs_parse_config_host, VfsParseConfigHost};

#[derive(Default)]
struct TestConfig {
    json_text: &'static str,
    config_file_name: &'static str,
    base_path: &'static str,
    all_file_list: &'static [(&'static str, &'static str)],
    existing_options: Option<CompilerOptions>,
}

#[derive(Default)]
struct ParseJsonConfigTestCase {
    title: &'static str,
    include_compiler_options: bool,
    input: Vec<TestConfig>,
}

const PARSE_CONFIG_FILE_TEXT_TO_JSON_TESTS: &[(&str, &[&str])] = &[
    ("returns empty config for file with only whitespaces", &["", " "]),
    ("returns empty config for file with comments only", &["// Comment", "/* Comment*/"]),
    ("returns empty config when config is empty object", &[r#"{}"#]),
    (
        "returns config object without comments",
        &[
            r#"{ // Excluded files
            "exclude": [
                // Exclude d.ts
                "file.d.ts"
            ]
        }"#,
            r#"{
            /* Excluded
                    Files
            */
            "exclude": [
                /* multiline comments can be in the middle of a line */"file.d.ts"
            ]
        }"#,
        ],
    ),
    (
        "keeps string content untouched",
        &[
            r#"{
            "exclude": [
                "xx//file.d.ts"
            ]
        }"#,
            r#"{
            "exclude": [
                "xx/*file.d.ts*/"
            ]
        }"#,
        ],
    ),
    (
        "handles escaped characters in strings correctly",
        &[
            r#"{
            "exclude": [
                "xx\"//files"
            ]
        }"#,
            r#"{
            "exclude": [
                "xx\\" // end of line comment
            ]
        }"#,
        ],
    ),
    (
        "returns object when users correctly specify library",
        &[
            r#"{
            "compilerOptions": {
                "lib": ["es5"]
            }
        }"#,
            r#"{
            "compilerOptions": {
                "lib": ["es5", "es6"]
            }
        }"#,
        ],
    ),
];

#[test]
fn test_parse_config_file_text_to_json() {
    let mut failures = Vec::new();
    for (title, input) in PARSE_CONFIG_FILE_TEXT_TO_JSON_TESTS {
        let mut baseline_content = String::new();
        for (i, json_text) in input.iter().enumerate() {
            baseline_content.push_str("Input::\n");
            baseline_content.push_str(json_text);
            baseline_content.push('\n');
            let (parsed, errors) = parse_config_file_text_to_json("/apath/tsconfig.json", Path::new("/apath"), json_text);
            baseline_content.push_str("Config::\n");
            baseline_content.push_str(&json::marshal_indent(&option_value_to_json(&parsed), "", "  ").unwrap());
            baseline_content.push('\n');
            baseline_content.push_str("Errors::\n");
            baseline_content.push_str(&format_diagnostics_with_color_and_context(
                &errors,
                "\n",
                &ComparePathsOptions { current_directory: "/".to_string(), use_case_sensitive_file_names: true },
            ));
            baseline_content.push('\n');
            if i != input.len() - 1 {
                baseline_content.push('\n');
            }
        }
        check_baseline(&mut failures, "config/tsconfigParsing", &format!("{title} jsonParse.js"), &baseline_content);
    }
    assert!(failures.is_empty(), "baseline mismatches: {failures:#?}");
}

// BEGIN GENERATED
const TSCONFIG_WITH_EXTENDS: &str = r#"{
  "files": ["/src/index.ts", "/src/app.ts"],
  "include": ["/src/**/*"],
  "exclude": [],
  "ts-node": {
    "compilerOptions": {
      "module": "commonjs"
    },
    "transpileOnly": true
  }
}"#;
const TSCONFIG_WITHOUT_CONFIG_DIR: &str = r#"{
  "compilerOptions": {
    "outDir": "bin"
  }
}"#;
const TSCONFIG_WITH_CONFIG_DIR: &str = r#"{
  "compilerOptions": {
    "outDir": "${configDir}/bin"
  }
}"#;
const TSCONFIG_WITH_EXTENDS_AND_CONFIG_DIR: &str = r#"{
  "compilerOptions": {
    "outFile": "${configDir}/outFile",
    "outDir": "${configDir}/outDir",
    "rootDir": "${configDir}/rootDir",
    "tsBuildInfoFile": "${configDir}/tsBuildInfoFile",
    "baseUrl": "${configDir}/baseUrl",
    "declarationDir": "${configDir}/declarationDir",
  }
}"#;

fn parse_json_config_file_tests() -> Vec<ParseJsonConfigTestCase> {
    vec![

	ParseJsonConfigTestCase {
		title: "ignore dotted files and folders",
		input: vec![TestConfig {
			json_text:       r#"{}"#,
			config_file_name: "tsconfig.json",
			base_path:       "/apath",
			all_file_list:    &[("/apath/test.ts", ""), ("/apath/.git/a.ts", ""), ("/apath/.b.ts", ""), ("/apath/..c.ts", "")],
			..Default::default()
		}],
		..Default::default()
	},
	ParseJsonConfigTestCase {
		title: "allow dotted files and folders when explicitly requested",
		input: vec![TestConfig {
			json_text: r#"{
                    "files": ["/apath/.git/a.ts", "/apath/.b.ts", "/apath/..c.ts"]
                }"#,
			config_file_name: "tsconfig.json",
			base_path:       "/apath",
			all_file_list:    &[("/apath/test.ts", ""), ("/apath/.git/a.ts", ""), ("/apath/.b.ts", ""), ("/apath/..c.ts", "")],
			..Default::default()
		}],
		..Default::default()
	},
	ParseJsonConfigTestCase {
		title: "implicitly exclude common package folders",
		input: vec![TestConfig {
			json_text:       r#"{}"#,
			config_file_name: "tsconfig.json",
			base_path:       "/",
			all_file_list:    &[("/node_modules/a.ts", ""), ("/bower_components/b.ts", ""), ("/jspm_packages/c.ts", ""), ("/d.ts", ""), ("/folder/e.ts", "")],
			..Default::default()
		}],
		..Default::default()
	},
	ParseJsonConfigTestCase {
		title: "generates errors for empty files list",
		input: vec![TestConfig {
			json_text: r#"{
                "files": []
            }"#,
			config_file_name: "/apath/tsconfig.json",
			base_path:       "/apath",
			all_file_list:    &[("/apath/a.ts", "")],
			..Default::default()
		}],
		..Default::default()
	},
	ParseJsonConfigTestCase {
		title: "generates errors for empty files list when no references are provided",
		input: vec![TestConfig {
			json_text: r#"{
                "files": [],
                "references": []
            }"#,
			config_file_name: "/apath/tsconfig.json",
			base_path:       "/apath",
			all_file_list:    &[("/apath/a.ts", "")],
			..Default::default()
		}],
		..Default::default()
	},
	ParseJsonConfigTestCase {
		title: "generates errors for directory with no .ts files",
		input: vec![TestConfig {
			json_text: r#"{
            }"#,
			config_file_name: "/apath/tsconfig.json",
			base_path:       "/apath",
			all_file_list:    &[("/apath/a.js", "")],
			..Default::default()
		}],
		..Default::default()
	},
	ParseJsonConfigTestCase {
		title: "generates errors for empty include",
		input: vec![TestConfig {
			json_text: r#"{
                "include": []
            }"#,
			config_file_name: "/apath/tsconfig.json",
			base_path:       "tests/cases/unittests",
			all_file_list:    &[("/apath/a.ts", "")],
			..Default::default()
		}],
		..Default::default()
	},
	ParseJsonConfigTestCase {
		title: "generates errors for include with parent directory after recursive wildcard",
		input: vec![TestConfig {
			json_text: r#"{
                "include": ["**/../*.ts"]
            }"#,
			config_file_name: "/apath/tsconfig.json",
			base_path:       "/apath",
			all_file_list:    &[("/apath/main.ts", "")],
			..Default::default()
		}],
		..Default::default()
	},
	ParseJsonConfigTestCase {
		title:                  "parses tsconfig with compilerOptions, files, include, and exclude",
		include_compiler_options: true,
		input: vec![TestConfig {
			json_text: r#"{
  "compilerOptions": {
    "outDir": "./dist",
    "strict": true,
    "noImplicitAny": true,
    "target": "ES2017",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "moduleDetection": "auto",
    "jsx": "react",
	"maxNodeModuleJsDepth": 1,
	"paths": {
      "jquery": ["./vendor/jquery/dist/jquery"]
    }
  },
  "files": ["/apath/src/index.ts", "/apath/src/app.ts"],
  "include": ["/apath/src/**/*"],
  "exclude": ["/apath/node_modules", "/apath/dist"]
}"#,
			config_file_name: "/apath/tsconfig.json",
			base_path:       "/apath",
			all_file_list:    &[("/apath/src/index.ts", ""), ("/apath/src/app.ts", ""), ("/apath/node_modules/module.ts", ""), ("/apath/dist/output.js", "")],
			..Default::default()
		}],
		..Default::default()
	},
	ParseJsonConfigTestCase {
		title: "generates errors when commandline option is in tsconfig",
		input: vec![TestConfig {
			json_text: r#"{
  "compilerOptions": {
    "help": true
  }
}"#,
			config_file_name: "/apath/tsconfig.json",
			base_path:       "/apath",
			all_file_list:    &[("/apath/a.ts", "")],
			..Default::default()
		}],
		..Default::default()
	},
	ParseJsonConfigTestCase {
		title: "does not generate errors for empty files list when one or more references are provided",
		input: vec![TestConfig {
			json_text: r#"{
                "files": [],
                "references": [{ "path": "/apath" }]
            }"#,
			config_file_name: "/apath/tsconfig.json",
			base_path:       "/apath",
			all_file_list:    &[("/apath/a.ts", "")],
			..Default::default()
		}],
		..Default::default()
	},
	ParseJsonConfigTestCase {
		title: "exclude outDir unless overridden",
		input: vec![TestConfig {
			json_text: r#"{
                "compilerOptions": {
                    "outDir": "bin"
                }
            }"#,
			config_file_name: "tsconfig.json",
			base_path:       "/",
			all_file_list:    &[("/bin/a.ts", ""), ("/b.ts", "")],
			..Default::default()
		}, TestConfig {
			json_text: r#"{
                "compilerOptions": {
                    "outDir": "bin"
                },
                "exclude": [ "obj" ]
            }"#,
			config_file_name: "tsconfig.json",
			base_path:       "/",
			all_file_list:    &[("/bin/a.ts", ""), ("/b.ts", "")],
			..Default::default()
		}],
		..Default::default()
	},
	ParseJsonConfigTestCase {
		title: "exclude declarationDir unless overridden",
		input: vec![TestConfig {
			json_text: r#"{
                "compilerOptions": {
                    "declarationDir": "declarations"
                }
            }"#,
			config_file_name: "tsconfig.json",
			base_path:       "/",
			all_file_list:    &[("/declarations/a.d.ts", ""), ("/a.ts", "")],
			..Default::default()
		}, TestConfig {
			json_text: r#"{
                "compilerOptions": {
                    "declarationDir": "declarations"
                },
                "exclude": [ "types" ]
            }"#,
			config_file_name: "tsconfig.json",
			base_path:       "/",
			all_file_list:    &[("/declarations/a.d.ts", ""), ("/a.ts", "")],
			..Default::default()
		}],
		..Default::default()
	},
	ParseJsonConfigTestCase {
		title: "generates errors for empty directory",
		input: vec![TestConfig {
			json_text: r#"{
                "compilerOptions": {
                    "allowJs": true
                }
            }"#,
			config_file_name: "/apath/tsconfig.json",
			base_path:       "/apath",
			all_file_list:    &[],
			..Default::default()
		}],
		..Default::default()
	},
	ParseJsonConfigTestCase {
		title: "generates errors for includes with outDir",
		input: vec![TestConfig {
			json_text: r#"{
                "compilerOptions": {
                    "outDir": "./"
                },
                "include": ["**/*"]
            }"#,
			config_file_name: "/apath/tsconfig.json",
			base_path:       "/apath",
			all_file_list:    &[("/apath/a.ts", "")],
			..Default::default()
		}],
		..Default::default()
	},
	ParseJsonConfigTestCase {
		title: "generates errors when include is not string",
		input: vec![TestConfig {
			json_text: r#"{
  "include": [
    [
      "./**/*.ts"
    ]
  ]
}"#,
			config_file_name: "/apath/tsconfig.json",
			base_path:       "/apath",
			all_file_list:    &[("/apath/a.ts", "")],
			..Default::default()
		}],
		..Default::default()
	},
	ParseJsonConfigTestCase {
		title: "generates errors when files is not string",
		input: vec![TestConfig {
			json_text: r#"{
  "files": [
    {
      "compilerOptions": {
        "experimentalDecorators": true,
        "allowJs": true
      }
    }
  ]
}"#,
			config_file_name: "/apath/tsconfig.json",
			base_path:       "/apath",
			all_file_list:    &[("/apath/a.ts", "")],
			..Default::default()
		}],
		..Default::default()
	},
	ParseJsonConfigTestCase {
		title: "with outDir from base tsconfig",
		input: vec![
			TestConfig {
				json_text: r#"{
  "extends": "./tsconfigWithoutConfigDir.json"
}"#,
				config_file_name: "tsconfig.json",
				base_path:       "/",
				all_file_list: &[("/tsconfigWithoutConfigDir.json", TSCONFIG_WITHOUT_CONFIG_DIR), ("/bin/a.ts", ""), ("/b.ts", "")],
				..Default::default()
			},
			TestConfig {
				json_text: r#"{
  "extends": "./tsconfigWithConfigDir.json"
}"#,
				config_file_name: "tsconfig.json",
				base_path:       "/",
				all_file_list: &[("/tsconfigWithConfigDir.json", TSCONFIG_WITH_CONFIG_DIR), ("/bin/a.ts", ""), ("/b.ts", "")],
				..Default::default()
			},
		],
		..Default::default()
	},
	ParseJsonConfigTestCase {
		title: "returns error when tsconfig have excludes",
		input: vec![TestConfig {
			json_text: r#"{
                    "compilerOptions": {
                        "lib": ["es5"]
                    },
                    "excludes": [
                        "foge.ts"
                    ]
                }"#,
			config_file_name: "tsconfig.json",
			base_path:       "/apath",
			all_file_list:    &[("/apath/test.ts", ""), ("/apath/foge.ts", "")],
			..Default::default()
		}],
		..Default::default()
	},
	ParseJsonConfigTestCase {
		title:                  "parses tsconfig with extends, files, include and other options",
		include_compiler_options: true,
		input: vec![TestConfig {
			json_text: r#"{
				"extends": "./tsconfigWithExtends.json",
				"compilerOptions": {
				    "outDir": "./dist",
    				"strict": true,
    				"noImplicitAny": true,
					"baseUrl": "",
				},
			}"#,
			config_file_name: "tsconfig.json",
			base_path:       "/",
			all_file_list:    &[("/tsconfigWithExtends.json", TSCONFIG_WITH_EXTENDS), ("/src/index.ts", ""), ("/src/app.ts", ""), ("/node_modules/module.ts", ""), ("/dist/output.js", "")],
			..Default::default()
		}],
		..Default::default()
	},
	ParseJsonConfigTestCase {
		title:                  "parses tsconfig with extends and configDir",
		include_compiler_options: true,
		input: vec![TestConfig {
			json_text: r#"{
				"extends": "./tsconfig.base.json"
			}"#,
			config_file_name: "tsconfig.json",
			base_path:       "/",
			all_file_list:    &[("/tsconfig.base.json", TSCONFIG_WITH_EXTENDS_AND_CONFIG_DIR), ("/src/index.ts", ""), ("/src/app.ts", ""), ("/node_modules/module.ts", ""), ("/dist/output.js", "")],
			..Default::default()
		}],
		..Default::default()
	},
	ParseJsonConfigTestCase {
		title: "reports error for an unknown option",
		input: vec![TestConfig {
			json_text: r#"{
			    "compilerOptions": {
				"unknown": true
			    }
			}"#,
			config_file_name: "tsconfig.json",
			base_path:       "/",
			all_file_list:    &[("/app.ts", "")],
			..Default::default()
		}],
		..Default::default()
	},
	ParseJsonConfigTestCase {
		title: "reports spelling suggestion for an unknown option",
		input: vec![TestConfig {
			json_text: r#"{
			    "compilerOptions": {
				"targt": 1
			    }
			}"#,
			config_file_name: "tsconfig.json",
			base_path:       "/",
			all_file_list:    &[("/app.ts", "")],
			..Default::default()
		}],
		..Default::default()
	},
	ParseJsonConfigTestCase {
		title: "reports errors for wrong type option and invalid enum value",
		input: vec![TestConfig {
			json_text: r#"{
			    "compilerOptions": {
				"target": "invalid value",
				"removeComments": "should be a boolean",
				"moduleResolution": "invalid value"
			    }
			}"#,
			config_file_name: "tsconfig.json",
			base_path:       "/",
			all_file_list:    &[("/app.ts", "")],
			..Default::default()
		}],
		..Default::default()
	},
	ParseJsonConfigTestCase {
		title:                  "reports errors for incorrectly cased option names",
		include_compiler_options: true,
		input: vec![TestConfig {
			json_text: r#"{
			    "compilerOptions": {
				"sourcemap": true,
				"declarationmap": true,
				"nouncheckedindexedaccess": true,
				"exactoptionalpropertytypes": true,
				"verbatimmodulesyntax": true,
				"isolatedmodules": true,
				"nouncheckedsideeffectimports": true,
				"moduledetection": "force",
				"skiplibcheck": true,
				"checkjs": true
			    }
			}"#,
			config_file_name: "tsconfig.json",
			base_path:       "/",
			all_file_list:    &[("/app.ts", "")],
			..Default::default()
		}],
		..Default::default()
	},
	ParseJsonConfigTestCase {
		title:                  "handles empty types array",
		include_compiler_options: true,
		input: vec![TestConfig {
			json_text: r#"{
			    "compilerOptions": {
					"types": []
				}
			}"#,
			config_file_name: "tsconfig.json",
			base_path:       "/",
			all_file_list:    &[("/app.ts", "")],
			..Default::default()
		}],
		..Default::default()
	},
	ParseJsonConfigTestCase {
		title:                  "issue 1267 scenario - extended files not picked up",
		include_compiler_options: true,
		input: vec![TestConfig {
			json_text: r#"{
  "extends": "./tsconfig-base/backend.json",
  "compilerOptions": {
    "baseUrl": "./",
    "outDir": "dist",
    "rootDir": "src",
    "resolveJsonModule": true
  },
  "exclude": ["node_modules", "dist"],
  "include": ["src/**/*"]
}"#,
			config_file_name: "tsconfig.json",
			base_path:       "/",
			all_file_list: &[("/tsconfig-base/backend.json", r#"{
  "$schema": "https://json.schemastore.org/tsconfig",
  "display": "Backend",
  "compilerOptions": {
    "allowJs": true,
    "module": "nodenext",
    "removeComments": true,
    "emitDecoratorMetadata": true,
    "experimentalDecorators": true,
    "allowSyntheticDefaultImports": true,
    "target": "esnext",
    "lib": ["ESNext"],
    "incremental": false,
    "esModuleInterop": true,
    "noImplicitAny": true,
    "moduleResolution": "nodenext",
    "types": ["node", "vitest/globals"],
    "sourceMap": true,
    "strictPropertyInitialization": false
  },
  "files": [
    "types/ical2json.d.ts",
    "types/express.d.ts",
    "types/multer.d.ts",
    "types/reset.d.ts",
    "types/stripe-custom-typings.d.ts",
    "types/nestjs-modules.d.ts",
    "types/luxon.d.ts",
    "types/nestjs-pino.d.ts"
  ],
  "ts-node": {
    "files": true
  }
}"#), ("/tsconfig-base/types/ical2json.d.ts", "export {}"), ("/tsconfig-base/types/express.d.ts", "export {}"), ("/tsconfig-base/types/multer.d.ts", "export {}"), ("/tsconfig-base/types/reset.d.ts", "export {}"), ("/tsconfig-base/types/stripe-custom-typings.d.ts", "export {}"), ("/tsconfig-base/types/nestjs-modules.d.ts", "export {}"), ("/tsconfig-base/types/luxon.d.ts", r#"declare module 'luxon' {
  interface TSSettings {
    throwOnInvalid: true
  }
}
export {}"#), ("/tsconfig-base/types/nestjs-pino.d.ts", "export {}"), ("/src/main.ts", "export {}"), ("/src/utils.ts", "export {}")],
			..Default::default()
		}],
		..Default::default()
	},
	ParseJsonConfigTestCase {
		title:                  "null overrides in extended tsconfig - array fields",
		include_compiler_options: true,
		input: vec![TestConfig {
			json_text: r#"{
  "extends": "./tsconfig-base.json",
  "compilerOptions": {
    "types": null,
    "lib": null,
    "typeRoots": null
  }
}"#,
			config_file_name: "tsconfig.json",
			base_path:       "/",
			all_file_list: &[("/tsconfig-base.json", r#"{
  "compilerOptions": {
    "types": ["node", "@types/jest"],
    "lib": ["es2020", "dom"],
    "typeRoots": ["./types", "./node_modules/@types"]
  }
}"#), ("/app.ts", "")],
			..Default::default()
		}],
		..Default::default()
	},
	ParseJsonConfigTestCase {
		title:                  "null overrides in extended tsconfig - string fields",
		include_compiler_options: true,
		input: vec![TestConfig {
			json_text: r#"{
  "extends": "./tsconfig-base.json",
  "compilerOptions": {
    "outDir": null,
    "baseUrl": null,
    "rootDir": null
  }
}"#,
			config_file_name: "tsconfig.json",
			base_path:       "/",
			all_file_list: &[("/tsconfig-base.json", r#"{
  "compilerOptions": {
    "outDir": "./dist",
    "baseUrl": "./src",
    "rootDir": "./src"
  }
}"#), ("/app.ts", "")],
			..Default::default()
		}],
		..Default::default()
	},
	ParseJsonConfigTestCase {
		title:                  "null overrides in extended tsconfig - mixed field types",
		include_compiler_options: true,
		input: vec![TestConfig {
			json_text: r#"{
  "extends": "./tsconfig-base.json",
  "compilerOptions": {
    "types": null,
    "outDir": null,
    "strict": false,
    "lib": ["es2022"],
    "allowJs": null
  }
}"#,
			config_file_name: "tsconfig.json",
			base_path:       "/",
			all_file_list: &[("/tsconfig-base.json", r#"{
  "compilerOptions": {
    "types": ["node"],
    "lib": ["es2020", "dom"],
    "outDir": "./dist",
    "strict": true,
    "allowJs": true,
    "target": "es2020"
  }
}"#), ("/app.ts", "")],
			..Default::default()
		}],
		..Default::default()
	},
	ParseJsonConfigTestCase {
		title:                  "null overrides with multiple extends levels",
		include_compiler_options: true,
		input: vec![TestConfig {
			json_text: r#"{
  "extends": "./tsconfig-middle.json",
  "compilerOptions": {
    "types": null,
    "lib": null
  }
}"#,
			config_file_name: "tsconfig.json",
			base_path:       "/",
			all_file_list: &[("/tsconfig-middle.json", r#"{
  "extends": "./tsconfig-base.json",
  "compilerOptions": {
    "types": ["jest"],
    "outDir": "./build"
  }
}"#), ("/tsconfig-base.json", r#"{
  "compilerOptions": {
    "types": ["node"],
    "lib": ["es2020"],
    "outDir": "./dist",
    "strict": true
  }
}"#), ("/app.ts", "")],
			..Default::default()
		}],
		..Default::default()
	},
	ParseJsonConfigTestCase {
		title:                  "null overrides in middle level of extends chain",
		include_compiler_options: true,
		input: vec![TestConfig {
			json_text: r#"{
  "extends": "./tsconfig-middle.json",
  "compilerOptions": {
    "outDir": "./final"
  }
}"#,
			config_file_name: "tsconfig.json",
			base_path:       "/",
			all_file_list: &[("/tsconfig-middle.json", r#"{
  "extends": "./tsconfig-base.json",
  "compilerOptions": {
    "types": null,
    "lib": null,
    "outDir": "./middle"
  }
}"#), ("/tsconfig-base.json", r#"{
  "compilerOptions": {
    "types": ["node"],
    "lib": ["es2020"],
    "outDir": "./base",
    "strict": true
  }
}"#), ("/app.ts", "")],
			..Default::default()
		}],
		..Default::default()
	},

    ]
}
// END GENERATED

type GetParsed = fn(&TestConfig, &'static VfsParseConfigHost, &str) -> ParsedCommandLine;

fn get_parsed_with_json_api(config: &TestConfig, host: &'static VfsParseConfigHost, base_path: &str) -> ParsedCommandLine {
    let config_file_name = tspath::get_normalized_absolute_path(config.config_file_name, base_path);
    let path = tspath::to_path(config.config_file_name, base_path, host.fs().use_case_sensitive_file_names());
    let (parsed, _) = parse_config_file_text_to_json(&config_file_name, path, config.json_text);
    parse_json_config_file_content(
        parsed,
        host,
        base_path,
        config.existing_options.as_ref(),
        &config_file_name,
        /*resolutionStack*/ &[],
        /*extendedConfigCache*/ None,
    )
}

fn get_parsed_with_json_source_file_api(config: &TestConfig, host: &'static VfsParseConfigHost, base_path: &str) -> ParsedCommandLine {
    let config_file_name = tspath::get_normalized_absolute_path(config.config_file_name, base_path);
    let path = tspath::to_path(config.config_file_name, base_path, host.fs().use_case_sensitive_file_names());
    let parsed = tsrs_parser::parse_source_file(
        SourceFileParseOptions { file_name: config_file_name.clone(), path, ..Default::default() },
        config.json_text,
        ScriptKind::JSON,
    );
    let ts_config_source_file = TsConfigSourceFile::new(parsed);
    parse_json_source_file_config_file_content(
        ts_config_source_file,
        host,
        host.get_current_directory(),
        config.existing_options.as_ref(),
        None,
        &config_file_name,
        /*resolutionStack*/ &[],
        /*extendedConfigCache*/ None,
    )
}

fn print_fs(output: &mut String, files: &dyn FS, root: &str) {
    walk_dir(files, root, &mut |path, entry, err| {
        if let Some(err) = err {
            return Err(err.clone().into());
        }
        if entry.unwrap().type_().is_regular() {
            let content = files.read_file(path).expect("failed to read file");
            output.push_str(&format!("//// [{path}]\r\n{content}\r\n\r\n"));
        }
        Ok(())
    })
    .unwrap();
}

fn baseline_parse_config_with(
    failures: &mut Vec<String>,
    baseline_file_name: &str,
    include_compiler_options: bool,
    input: &[TestConfig],
    get_parsed: GetParsed,
) {
    let mut baseline_content = String::new();
    for (i, config) in input.iter().enumerate() {
        let mut base_path = config.base_path.to_string();
        if base_path.is_empty() {
            base_path = tspath::get_normalized_absolute_path(&tspath::get_directory_path(config.config_file_name), "");
        }
        let config_file_name = tspath::combine_paths(&base_path, &[config.config_file_name]);
        let mut all_file_lists: Vec<(&str, &str)> = config.all_file_list.iter().filter(|(k, _)| *k != config_file_name).copied().collect();
        all_file_lists.push((&config_file_name, config.json_text));
        let host = new_vfs_parse_config_host(&all_file_lists, config.base_path, true /*useCaseSensitiveFileNames*/);
        let parsed_config_file_content = get_parsed(config, host, &base_path);

        baseline_content.push_str("Fs::\n");
        print_fs(&mut baseline_content, host.fs(), "/");
        baseline_content.push('\n');
        baseline_content.push_str("configFileName:: ");
        baseline_content.push_str(config.config_file_name);
        baseline_content.push('\n');
        if include_compiler_options {
            baseline_content.push_str("CompilerOptions::\n");
            let options = match parsed_config_file_content.parsed_config.compiler_options {
                Some(options) => compiler_options_to_json(&options),
                None => json::Value::Null,
            };
            baseline_content.push_str(&json::marshal_indent(&options, "", "  ").unwrap());
            baseline_content.push('\n');
            baseline_content.push('\n');

            if let Some(type_acquisition) = &parsed_config_file_content.parsed_config.type_acquisition {
                baseline_content.push_str("TypeAcquisition::\n");
                baseline_content.push_str(&json::marshal_indent(&type_acquisition_to_json(type_acquisition), "", "  ").unwrap());
                baseline_content.push('\n');
                baseline_content.push('\n');
            }
        }
        baseline_content.push_str("FileNames::\n");
        baseline_content.push_str(&parsed_config_file_content.parsed_config.file_names.join(","));
        baseline_content.push('\n');
        baseline_content.push_str("Errors::\n");
        baseline_content.push_str(&format_diagnostics_with_color_and_context(
            &parsed_config_file_content.errors,
            "\r\n",
            &ComparePathsOptions { current_directory: base_path.clone(), use_case_sensitive_file_names: true },
        ));
        baseline_content.push('\n');
        if i != input.len() - 1 {
            baseline_content.push('\n');
        }
    }
    check_baseline(failures, "config/tsconfigParsing", baseline_file_name, &baseline_content);
}

#[test]
fn test_parse_json_config_file_content() {
    let mut failures = Vec::new();
    for rec in parse_json_config_file_tests() {
        baseline_parse_config_with(
            &mut failures,
            &format!("{} with json api.js", rec.title),
            rec.include_compiler_options,
            &rec.input,
            get_parsed_with_json_api,
        );
    }
    assert!(failures.is_empty(), "baseline mismatches: {failures:#?}");
}

#[test]
fn test_parse_json_source_file_config_file_content() {
    let mut failures = Vec::new();
    for rec in parse_json_config_file_tests() {
        baseline_parse_config_with(
            &mut failures,
            &format!("{} with jsonSourceFile api.js", rec.title),
            rec.include_compiler_options,
            &rec.input,
            get_parsed_with_json_source_file_api,
        );
    }
    assert!(failures.is_empty(), "baseline mismatches: {failures:#?}");
}

fn object(entries: Vec<(&str, CompilerOptionsValue)>) -> CompilerOptionsValue {
    let mut m = OrderedMap::default();
    for (k, v) in entries {
        m.insert(k.to_string(), v);
    }
    CompilerOptionsValue::Object(m)
}

// Go also passes plain maps and typed slices; the Rust value type only has the normalized JSON shapes.
#[test]
fn test_parse_json_config_file_content_accepts_json_representations() {
    let host = new_vfs_parse_config_host(&[("/project/index.ts", "export {};")], "/project", true /*useCaseSensitiveFileNames*/);

    let (ordered_map, parse_errors) = parse_config_file_text_to_json(
        "/project/tsconfig.json",
        Path::new("/project/tsconfig.json"),
        r#"{"compilerOptions":{"strict":true},"files":["index.ts"]}"#,
    );
    assert_eq!(parse_errors.len(), 0);

    let ordered_map_with_typed_slices = object(vec![
        ("compilerOptions", object(vec![("strict", CompilerOptionsValue::Bool(true))])),
        ("files", CompilerOptionsValue::StringArray(vec!["index.ts".to_string()])),
    ]);

    for json in [ordered_map, ordered_map_with_typed_slices] {
        let parsed = parse_json_config_file_content(json, host, "/project", None, "/project/tsconfig.json", &[], None);
        assert_eq!(parsed.file_names(), &["/project/index.ts".to_string()]);
        assert!(parsed.compiler_options().unwrap().strict.is_true());
        assert_eq!(parsed.errors.len(), 0);
    }
}

#[test]
fn test_parse_json_config_file_content_preserves_raw() {
    let host = new_vfs_parse_config_host(&[("/project/index.ts", "export {};")], "/project", true /*useCaseSensitiveFileNames*/);

    // Go sorts the keys of a plain map; pass them in that order.
    let parsed = parse_json_config_file_content(
        object(vec![
            ("compileOnSave", CompilerOptionsValue::Bool(true)),
            ("customSetting", object(vec![("enabled", CompilerOptionsValue::Bool(true))])),
            ("files", CompilerOptionsValue::Array(vec![CompilerOptionsValue::String("index.ts".to_string())])),
        ]),
        host,
        "/project",
        None,
        "/project/tsconfig.json",
        &[],
        None,
    );

    assert_eq!(parsed.errors.len(), 0);
    assert_eq!(parsed.compile_on_save, Some(true));

    let raw = parsed.raw.as_object().unwrap();
    assert_eq!(raw.keys().cloned().collect::<Vec<_>>(), vec!["compileOnSave", "customSetting", "files"]);
    assert!(raw.contains_key("customSetting"));
}

#[test]
fn test_parse_json_config_file_content_handles_null_array_elements() {
    let host = new_vfs_parse_config_host(&[("/project/index.ts", "export {};")], "/project", true /*useCaseSensitiveFileNames*/);
    for property in ["files", "include", "exclude"] {
        let parsed = parse_json_config_file_content(
            object(vec![(property, CompilerOptionsValue::Array(vec![CompilerOptionsValue::Null]))]),
            host,
            "/project",
            None,
            "/project/tsconfig.json",
            &[],
            None,
        );
        assert!(!parsed.errors.is_empty(), "{property}");
        assert_eq!(parsed.errors[0].code(), diagnostics::Compiler_option_0_requires_a_value_of_type_1.code());
    }
}

#[test]
fn test_parse_json_config_file_content_defaults_compile_on_save_to_false() {
    let host = new_vfs_parse_config_host(&[("/project/index.ts", "export {};")], "/project", true /*useCaseSensitiveFileNames*/);
    let parsed = parse_json_config_file_content(
        object(vec![("files", CompilerOptionsValue::Array(vec![CompilerOptionsValue::String("index.ts".to_string())]))]),
        host,
        "/project",
        None,
        "/project/tsconfig.json",
        &[],
        None,
    );
    assert_eq!(parsed.compile_on_save, Some(false));
}

#[test]
fn test_parse_json_source_file_config_file_content_reports_invalid_extended_config() {
    let files: &[(&str, &str)] = &[
        (
            "/project/tsconfig.json",
            r#"{
  "extends": "./bad.json"
}"#,
        ),
        // The parser recovers from this as object-like JSON, producing expected-token errors for ':', ',', ',', and '}'.
        ("/project/bad.json", "{ this is not json"),
        ("/project/main.ts", "export const x = 1;"),
    ];
    let host = new_vfs_parse_config_host(files, "/project", true /*useCaseSensitiveFileNames*/);
    let config_file_name = "/project/tsconfig.json";
    let config_file = new_tsconfig_source_file_from_file_path(
        config_file_name,
        tspath::to_path(config_file_name, host.get_current_directory(), host.fs().use_case_sensitive_file_names()),
        files[0].1,
    );

    let parsed = parse_json_source_file_config_file_content(config_file, host, host.get_current_directory(), None, None, config_file_name, &[], None);

    let parse_errors: Vec<P<Diagnostic>> =
        parsed.errors.iter().copied().filter(|diagnostic| diagnostic.code() == diagnostics::X_0_expected.code()).collect();
    let expected_parse_error_messages = [":", ",", ",", "}"];
    let expected_parse_error_positions = [7, 10, 14, 18];
    assert_eq!(expected_parse_error_messages.len(), parse_errors.len());
    assert_eq!(parse_errors.iter().map(|d| d.message_args()[0].clone()).collect::<Vec<_>>(), expected_parse_error_messages);
    assert_eq!(parse_errors.iter().map(|d| d.pos()).collect::<Vec<_>>(), expected_parse_error_positions);
    for diagnostic in &parse_errors {
        assert_eq!(diagnostic.file().unwrap().file_name(), "/project/bad.json");
    }
}

// Extending an empty config file used to panic on nil Statements (#4265).
#[test]
fn test_parse_json_source_file_config_file_content_with_empty_extended_config() {
    let files: &[(&str, &str)] = &[
        (
            "/project/tsconfig.json",
            r#"{
  "extends": "./base.json"
}"#,
        ),
        ("/project/base.json", ""),
        ("/project/main.ts", "export const x = 1;"),
    ];
    let host = new_vfs_parse_config_host(files, "/project", true /*useCaseSensitiveFileNames*/);
    let config_file_name = "/project/tsconfig.json";
    let config_file = new_tsconfig_source_file_from_file_path(
        config_file_name,
        tspath::to_path(config_file_name, host.get_current_directory(), host.fs().use_case_sensitive_file_names()),
        files[0].1,
    );

    let parsed = parse_json_source_file_config_file_content(config_file, host, host.get_current_directory(), None, None, config_file_name, &[], None);
    assert_eq!(parsed.file_names(), &["/project/main.ts".to_string()]);
}

fn line_and_character(diagnostic: P<Diagnostic>) -> (usize, usize) {
    let text = diagnostic.file().unwrap().text();
    let before = &text[..diagnostic.pos() as usize];
    let line = before.matches('\n').count();
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    (line, before[line_start..].encode_utf16().count())
}

#[test]
fn test_parse_json_source_file_config_file_content_does_not_duplicate_unquoted_key_diagnostics() {
    let parsed = get_parsed_command_line(
        r#"{
  compilerOptions: {
    strict: true
  }
}"#,
        &[("/main.ts", "export const x = 1;")],
        "/",
        true, /*useCaseSensitiveFileNames*/
    );

    let diags = parsed.get_config_file_parsing_diagnostics();
    assert_eq!(diags.len(), 2);
    let expected_locations = [(1, 2), (2, 4)];
    for (index, diagnostic) in diags.iter().enumerate() {
        assert_eq!(diagnostic.code(), diagnostics::String_literal_with_double_quotes_expected.code());
        assert_eq!(line_and_character(*diagnostic), expected_locations[index]);
    }
}

#[test]
fn test_parse_json_source_file_config_file_content_reports_question_token_diagnostics() {
    let parsed = get_parsed_command_line(
        r#"{
  compilerOptions?: {
    strict?: true
  }
}"#,
        &[("/main.ts", "export const x = 1;")],
        "/",
        true, /*useCaseSensitiveFileNames*/
    );

    let question_token_diagnostics: Vec<P<Diagnostic>> = parsed
        .get_config_file_parsing_diagnostics()
        .into_iter()
        .filter(|d| d.code() == diagnostics::The_0_modifier_can_only_be_used_in_TypeScript_files.code())
        .collect();
    assert_eq!(question_token_diagnostics.len(), 2);
    let expected_locations = [(1, 17), (2, 10)];
    for (index, diagnostic) in question_token_diagnostics.iter().enumerate() {
        assert_eq!(line_and_character(*diagnostic), expected_locations[index]);
    }
}

#[test]
fn test_parse_null_enum_compiler_options() {
    let config = TestConfig {
        json_text: r#"{
			"compilerOptions": {
				"target": null,
				"module": null
			}
		}"#,
        config_file_name: "tsconfig.json",
        base_path: "/",
        all_file_list: &[("/app.ts", "")],
        ..Default::default()
    };
    for get_parsed in [get_parsed_with_json_api as GetParsed, get_parsed_with_json_source_file_api] {
        let mut all_file_lists: Vec<(&str, &str)> = config.all_file_list.to_vec();
        all_file_lists.push(("/tsconfig.json", config.json_text));
        let host = new_vfs_parse_config_host(&all_file_lists, config.base_path, true /*useCaseSensitiveFileNames*/);
        let parsed_config_file_content = get_parsed(&config, host, config.base_path);
        assert_eq!(parsed_config_file_content.errors.len(), 0);
    }
}

#[test]
fn test_parse_type_acquisition() {
    let cases: &[(&str, &str, &str)] = &[
        (
            "Convert correctly format tsconfig.json to typeAcquisition ",
            r#"{
	"typeAcquisition": {
		"enable": true,
		"include": ["0.d.ts", "1.d.ts"],
		"exclude": ["0.js", "1.js"],
	},
}"#,
            "tsconfig.json",
        ),
        (
            "Convert incorrect format tsconfig.json to typeAcquisition ",
            r#"{
	"typeAcquisition": {
		"enableAutoDiscovy": true,
	}
}"#,
            "tsconfig.json",
        ),
        ("Convert default tsconfig.json to typeAcquisition ", r#"{}"#, "tsconfig.json"),
        (
            "Convert tsconfig.json with only enable property to typeAcquisition ",
            r#"{
	"typeAcquisition": {
		"enable": true,
	},
}"#,
            "tsconfig.json",
        ),
        // jsconfig.json
        (
            "Convert jsconfig.json to typeAcquisition ",
            r#"{
	"typeAcquisition": {
		"enable": false,
		"include": ["0.d.ts"],
		"exclude": ["0.js"],
	},
}"#,
            "jsconfig.json",
        ),
        ("Convert default jsconfig.json to typeAcquisition ", r#"{}"#, "jsconfig.json"),
        (
            "Convert incorrect format jsconfig.json to typeAcquisition ",
            r#"{
	"typeAcquisition": {
		"enableAutoDiscovy": true,
	},
}"#,
            "jsconfig.json",
        ),
        (
            "Convert jsconfig.json with only enable property to typeAcquisition ",
            r#"{
	"typeAcquisition": {
		"enable": false,
	},
}"#,
            "jsconfig.json",
        ),
    ];
    let mut failures = Vec::new();
    for (title, config, config_name) in cases {
        let input = vec![TestConfig {
            json_text: config,
            config_file_name: config_name,
            base_path: "/apath",
            all_file_list: &[("/apath/a.ts", ""), ("/apath/b.ts", "")],
            ..Default::default()
        }];
        baseline_parse_config_with(&mut failures, &format!("{title} with json api.js"), true, &input, get_parsed_with_json_api);
        baseline_parse_config_with(
            &mut failures,
            &format!("{title} with jsonSourceFile api.js"),
            true,
            &input,
            get_parsed_with_json_source_file_api,
        );
    }
    assert!(failures.is_empty(), "baseline mismatches: {failures:#?}");
}

fn parse_src_compiler() -> ParsedCommandLine {
    let compiler_dir = tspath::normalize_slashes(&repo_root().join("ts-ref/tsc/testdata/fixtures/compiler").to_string_lossy());
    let tsconfig_file_name = tspath::combine_paths(&compiler_dir, &["tsconfig.json"]);
    let fs = tsrs_vfs::osvfs::fs();
    let host: &'static VfsParseConfigHost =
        P::new(VfsParseConfigHost { vfs: fs, current_directory: compiler_dir.clone() }).get();
    let json_text = fs.read_file(&tsconfig_file_name).unwrap();
    let config_file = new_tsconfig_source_file_from_file_path(
        &tsconfig_file_name,
        tspath::to_path(&tsconfig_file_name, &compiler_dir, fs.use_case_sensitive_file_names()),
        &json_text,
    );
    let parsed =
        parse_json_source_file_config_file_content(config_file, host, host.get_current_directory(), None, None, &tsconfig_file_name, &[], None);
    assert_eq!(parsed.errors.len(), 0, "Expected no errors in parsed command line");
    parsed
}

#[test]
fn test_parse_src_compiler() {
    let parsed = parse_src_compiler();
    let opts = parsed.compiler_options().unwrap();
    assert_eq!(opts.types, Some(Vec::new()));
    assert_eq!(opts.module, ModuleKind::NodeNext);
    assert_eq!(opts.module_resolution, ModuleResolutionKind::NodeNext);
    assert_eq!(opts.target, ScriptTarget::ES2020);
    assert_eq!(parsed.file_names().len(), 79);
    for file in ["checker.ts", "diagnosticInformationMap.generated.ts", "node.d.ts", "program.ts"] {
        assert!(parsed.file_names().contains(&tspath::combine_paths(&tspath::get_directory_path(&opts.config_file_path), &[file])));
    }
}

// memoCache is a minimal memoizing ExtendedConfigCache used by tests to simulate
// cache hits across multiple parses of configs that extend a common base.
#[derive(Default)]
struct MemoCache {
    m: std::sync::Mutex<rustc_hash::FxHashMap<Path, P<ExtendedConfigCacheEntry>>>,
}

#[test]
fn parsed_config_and_cache_do_not_retain_a_scoped_host() {
    struct ScopedHost {
        fs: std::sync::Arc<dyn FS>,
    }
    impl ParseConfigHost for ScopedHost {
        fn fs(&self) -> &dyn FS {
            &*self.fs
        }
        fn get_current_directory(&self) -> &str {
            "/project"
        }
    }

    let cache = MemoCache::default();
    let (parsed, fs_owner) = {
        let files = [
            ("/project/tsconfig.json", r#"{"extends":"preset","files":["index.ts"]}"#),
            ("/project/index.ts", "export const value = 1;"),
            ("/project/node_modules/preset/package.json", r#"{"name":"preset","tsconfig":"base.json"}"#),
            ("/project/node_modules/preset/base.json", r#"{"extends":"./nested.json"}"#),
            ("/project/node_modules/preset/nested.json", r#"{"compilerOptions":{"strict":true}}"#),
        ];
        let fs: std::sync::Arc<dyn FS> = std::sync::Arc::new(tsrs_vfs::vfstest::from_map(files.into_iter().map(|(name, text)| (name, text.to_string())), true));
        let fs_owner = std::sync::Arc::downgrade(&fs);
        let host = ScopedHost { fs };
        for _ in 0..2 {
            let (parsed, errors) = get_parsed_command_line_of_config_file("/project/tsconfig.json", None, None, &host, Some(&cache));
            assert!(errors.is_empty());
            assert!(parsed.unwrap().errors.is_empty());
        }
        let (parsed, errors) = get_parsed_command_line_of_config_file("/project/tsconfig.json", None, None, &host, Some(&cache));
        assert!(errors.is_empty());
        (parsed.unwrap(), fs_owner)
    };
    assert!(fs_owner.upgrade().is_none(), "config parsing retained the host filesystem");
    assert!(parsed.errors.is_empty());
    assert!(parsed.compiler_options().unwrap().strict.is_true());
    assert_eq!(parsed.file_names(), &["/project/index.ts"]);
}

impl ExtendedConfigCache for MemoCache {
    fn get_extended_config(
        &self,
        file_name: &str,
        path: &Path,
        resolution_stack: &[Path],
        host: &dyn ParseConfigHost,
    ) -> P<ExtendedConfigCacheEntry> {
        if let Some(e) = self.m.lock().unwrap().get(path) {
            return *e;
        }
        let e = parse_extended_config(file_name, path.clone(), resolution_stack, host, Some(self));
        self.m.lock().unwrap().insert(path.clone(), e);
        e
    }
}

fn parse_config_with_cache(host: &'static VfsParseConfigHost, config_file_name: &str, cache: &MemoCache) -> ParsedCommandLine {
    let cfg_path = tspath::to_path(config_file_name, host.get_current_directory(), host.fs().use_case_sensitive_file_names());
    let json_text = host.fs().read_file(config_file_name).unwrap_or_else(|| panic!("missing {config_file_name} in test fs"));
    let ts_config_source_file = TsConfigSourceFile::new(tsrs_parser::parse_source_file(
        SourceFileParseOptions { file_name: config_file_name.to_string(), path: cfg_path, ..Default::default() },
        &json_text,
        ScriptKind::JSON,
    ));
    parse_json_source_file_config_file_content(
        ts_config_source_file,
        host,
        host.get_current_directory(),
        None,
        None,
        config_file_name,
        &[],
        Some(cache),
    )
}

// TestExtendedConfigErrorsAppearOnCacheHit verifies that diagnostics produced while parsing an
// extended config are still reported when the extended config comes from the cache.
#[test]
fn test_extended_config_errors_appear_on_cache_hit() {
    // single config parsed twice
    let files: &[(&str, &str)] = &[
        (
            "/tsconfig.json",
            r#"{
  "extends": "./base.json"
}"#,
        ),
        // 'excludes' instead of 'exclude' triggers diagnostic
        (
            "/base.json",
            r#"{
  "excludes": ["**/*.ts"]
}"#,
        ),
        ("/app.ts", "export {}"),
    ];
    let host = new_vfs_parse_config_host(files, "/", true /*useCaseSensitiveFileNames*/);
    let cache = MemoCache::default();
    let first = parse_config_with_cache(host, "/tsconfig.json", &cache);
    assert!(!first.errors.is_empty(), "expected diagnostics on first parse, got 0");
    let second = parse_config_with_cache(host, "/tsconfig.json", &cache);
    assert!(!second.errors.is_empty(), "expected diagnostics on second parse (cache hit), got 0");

    // two configs share same base
    let files: &[(&str, &str)] = &[
        (
            "/base.json",
            r#"{
  "excludes": ["**/*.ts"]
}"#,
        ),
        (
            "/projA/tsconfig.json",
            r#"{
  "extends": "../base.json"
}"#,
        ),
        (
            "/projB/tsconfig.json",
            r#"{
  "extends": "../base.json"
}"#,
        ),
        ("/projA/app.ts", "export {}"),
        ("/projB/app.ts", "export {}"),
    ];
    let host = new_vfs_parse_config_host(files, "/", true /*useCaseSensitiveFileNames*/);
    let cache = MemoCache::default();
    let first = parse_config_with_cache(host, "/projA/tsconfig.json", &cache);
    assert!(!first.errors.is_empty(), "expected diagnostics for projA parse, got 0");
    let second = parse_config_with_cache(host, "/projB/tsconfig.json", &cache);
    assert!(!second.errors.is_empty(), "expected diagnostics for projB parse (cache hit on base), got 0");
}

#[test]
fn test_extended_config_config_dir_paths_are_not_cached() {
    let files: &[(&str, &str)] = &[
        (
            "/tsconfig.base.json",
            r#"{
  "compilerOptions": {
    "paths": {
      "@pkg/*": ["${configDir}/src/*"]
    }
  }
}"#,
        ),
        (
            "/packages/a/tsconfig.json",
            r#"{
  "extends": "../../tsconfig.base.json"
}"#,
        ),
        (
            "/packages/b/tsconfig.json",
            r#"{
  "extends": "../../tsconfig.base.json"
}"#,
        ),
        ("/packages/a/index.ts", "export {}"),
        ("/packages/b/index.ts", "export {}"),
    ];

    let host = new_vfs_parse_config_host(files, "/", true /*useCaseSensitiveFileNames*/);
    let cache = MemoCache::default();

    let parse_config = |config_file_name: &str| -> ParsedCommandLine {
        let (parsed, errors) = get_parsed_command_line_of_config_file(config_file_name, None, None, host, Some(&cache));
        assert!(errors.is_empty(), "unexpected errors parsing {config_file_name}: {errors:?}");
        parsed.unwrap()
    };

    parse_config("/packages/a/tsconfig.json");
    let parsed = parse_config("/packages/b/tsconfig.json");
    let paths = parsed.compiler_options().unwrap().paths.clone().unwrap();
    assert_eq!(paths.get("@pkg/*").cloned().unwrap_or_default(), vec!["/packages/b/src/*".to_string()]);
}

#[test]
fn test_custom_conditions_null_override() {
    let files: &[(&str, &str)] = &[
        (
            "/project/tsconfig.json",
            r#"{
  "compilerOptions": {
    "customConditions": ["condition1", "condition2"]
  }
}"#,
        ),
        ("/project/index.ts", r#"console.log("Hello, World!");"#),
    ];

    let host = new_vfs_parse_config_host(files, "/project", true);

    // Parse command line with --customConditions null
    let cmd_line = parse_command_line(
        &["--project", "/project", "--customConditions", "null"].iter().map(|s| s.to_string()).collect::<Vec<_>>(),
        host,
    );

    // Check that the raw options contain null for customConditions
    let raw_map = cmd_line.raw.as_object().expect("Raw options should be an OrderedMap");
    let custom_conditions_raw = raw_map.get("customConditions");
    assert!(custom_conditions_raw.is_some(), "customConditions should exist in raw options");
    assert!(custom_conditions_raw.unwrap().is_null(), "customConditions should be nil in raw options");

    // Now parse the config file with the command line options
    // Wrap command line options in "compilerOptions" key to match tsconfig.json structure
    let wrapped_raw = object(vec![("compilerOptions", cmd_line.raw.clone())]);
    let (parsed_config, errors) = get_parsed_command_line_of_config_file(
        "/project/tsconfig.json",
        cmd_line.compiler_options().as_deref(),
        Some(&wrapped_raw),
        host,
        None,
    );

    assert!(errors.is_empty(), "Should not have errors: {errors:?}");

    // Check that customConditions is nil (overridden by command line)
    let custom_conditions = parsed_config.unwrap().compiler_options().unwrap().custom_conditions.clone();
    assert!(custom_conditions.is_none(), "customConditions should be nil after override, got: {custom_conditions:?}");
}

// Real-world smoke test: TSRS_TSOPTIONS_SMOKE_DIR=<project dir> cargo test -p tsrs_tsoptions -- --ignored smoke
// Writes the root file names (one per line) to TSRS_TSOPTIONS_SMOKE_OUT when set.
#[test]
#[ignore]
fn smoke_parse_project_from_env() {
    let Ok(dir) = std::env::var("TSRS_TSOPTIONS_SMOKE_DIR") else {
        return;
    };
    let dir = tspath::normalize_slashes(&dir);
    let fs = tsrs_vfs::osvfs::fs();
    let host: &'static VfsParseConfigHost = P::new(VfsParseConfigHost { vfs: fs, current_directory: dir.clone() }).get();
    let start = std::time::Instant::now();
    let (parsed, errors) = get_parsed_command_line_of_config_file(&tspath::combine_paths(&dir, &["tsconfig.json"]), None, None, host, None);
    assert!(errors.is_empty(), "{errors:?}");
    let parsed = parsed.unwrap();
    eprintln!("parsed in {:?}", start.elapsed());
    for e in parsed.get_config_file_parsing_diagnostics() {
        eprintln!("error TS{}: {}", e.code(), e.localize());
    }
    eprintln!("files: {} extended: {:?}", parsed.file_names().len(), parsed.extended_source_files());
    if let Ok(out) = std::env::var("TSRS_TSOPTIONS_SMOKE_OUT") {
        std::fs::write(out, parsed.file_names().join("\n") + "\n").unwrap();
    }
}
