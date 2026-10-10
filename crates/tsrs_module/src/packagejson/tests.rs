use super::json::{self, Json};
use super::*;

fn member<'a>(value: &'a Json, name: &str) -> &'a Json {
    let Json::Object(members) = value else { panic!("not an object") };
    &members.iter().rev().find(|(n, _)| n == name).expect("member").1
}

#[test]
fn test_expected() {
    let p = parse(
        r##"{
		"name": "test",
		"version": 2,
		"exports": null
	}"##,
    )
    .unwrap();

    assert!(p.name.valid);
    assert_eq!(p.name.value, "test");

    assert!(!p.version.valid);
    assert_eq!(p.version.value, "");
    assert_eq!(p.version.actual_json_type(), "number");
    assert_eq!(p.version.expected_json_type(), "string");

    assert_eq!(p.exports.type_(), JSONValueType::Null);

    let mut exports: Expected<String> = Expected::default();
    exports.unmarshal_json(&Json::Null);
    assert!(exports.null);
    assert!(!exports.valid);

    assert!(!p.main.valid);
    assert!(!p.main.null);
    assert!(!p.main.is_present());
    assert_eq!(p.main.value, "");
}

#[test]
fn test_exports() {
    let e = parse(
        r##"{
		"imports": {
			"#foo": {
				"import": "./foo.ts"
			}
		},
		"exports": {
			".": {
				"import": "./test.ts",
				"default": "./test.ts"
			},
			"./test": [
				"./test1.ts",
				"./test2.ts",
				null
			],
			"./null": null
		}
	}"##,
    )
    .unwrap();

    assert!(e.exports.is_subpaths());
    assert_eq!(e.exports.as_object().len(), 3);
    assert!(e.exports.as_object()["."].is_conditions());
    assert_eq!(e.exports.as_object()["."].as_object()["import"].type_(), JSONValueType::String);
    assert_eq!(e.exports.as_object()["./test"].as_array()[2].type_(), JSONValueType::Null);
    assert_eq!(e.exports.as_object()["./null"].type_(), JSONValueType::Null);

    assert!(e.imports.is_imports());
    assert_eq!(e.imports.as_object().len(), 1);
    assert!(e.imports.as_object()["#foo"].is_conditions());
    assert_eq!(e.imports.as_object()["#foo"].as_object()["import"].type_(), JSONValueType::String);
}

#[test]
fn test_json_value() {
    let raw = json::parse(
        r##"{
		"private": true,
		"false": false,
		"name": "test",
		"version": 2,
		"exports": {
			".": {
				"import": "./test.ts",
				"default": "./test.ts"
			},
			"./test": [
				"./test1.ts",
				"./test2.ts",
				null
			],
			"./null": null
		},
		"imports": null
	}"##,
    )
    .unwrap();
    let get = |name: &str| JSONValue::from_json(member(&raw, name));

    assert_eq!(get("private").data, JSONData::Boolean(true));
    assert!(!get("private").is_falsy());
    assert!(get("false").is_falsy());
    assert_eq!(get("name").data, JSONData::String("test".to_string()));
    assert_eq!(get("version").data, JSONData::Number(2.0));
    // Go compares the float64 against an int-typed zero, so numbers are never falsy.
    assert!(!JSONValue::from_json(&Json::Number(0.0)).is_falsy());

    let exports = get("exports");
    assert_eq!(exports.type_(), JSONValueType::Object);
    assert_eq!(exports.as_object().len(), 3);
    assert_eq!(exports.as_object()["."].type_(), JSONValueType::Object);
    assert_eq!(exports.as_object()["."].as_object()["import"].as_string(), "./test.ts");
    assert_eq!(exports.as_object()["./test"].type_(), JSONValueType::Array);
    assert_eq!(exports.as_object()["./test"].as_array().len(), 3);
    assert_eq!(exports.as_object()["./test"].as_array()[0].as_string(), "./test1.ts");
    assert_eq!(exports.as_object()["./test"].as_array()[1].as_string(), "./test2.ts");
    assert_eq!(exports.as_object()["./test"].as_array()[2].type_(), JSONValueType::Null);
    assert_eq!(exports.as_object()["./null"].type_(), JSONValueType::Null);

    assert_eq!(get("imports").type_(), JSONValueType::Null);
    assert_eq!(JSONValue::default().type_(), JSONValueType::NotPresent);
}

#[test]
fn test_parse() {
    let got = parse(
        r##"{
				"name": "test-package",
				"name": "test-package",
				"version": "1.0.0"
			}"##,
    )
    .unwrap();
    assert_eq!(got, Fields { name: expected_of("test-package".to_string()), version: expected_of("1.0.0".to_string()), ..Default::default() });

    let got = parse(
        r##"{
				"name": "test-package",
				"typescript": {
					"contentMapper": { "exec": ["mapper"], "dynamicConfig": true }
				}
			}"##,
    )
    .unwrap();
    assert_eq!(
        got,
        Fields {
            name: expected_of("test-package".to_string()),
            content_mapper: expected_of(ContentMapperFields {
                exec: expected_of(vec!["mapper".to_string()]),
                dynamic_config: expected_of(true),
                ..Default::default()
            }),
            ..Default::default()
        }
    );

    let got = parse(r##"{ "name": "test-package", "typescript": "invalid" }"##).unwrap();
    assert_eq!(got, Fields { name: expected_of("test-package".to_string()), ..Default::default() });
}

#[test]
fn test_parse_errors_and_dependencies() {
    assert!(parse("[]").is_err());
    assert!(parse("{").is_err());
    assert_eq!(parse("null").unwrap(), Fields::default());

    let got = parse(r##"{ "dependencies": { "a": "1", "b": "2" }, "peerDependencies": { "c": 1 }, "devDependencies": { "d": "1", "d": "2" } }"##).unwrap();
    assert!(got.dependencies.valid);
    assert!(got.has_dependency("a"));
    assert!(!got.peer_dependencies.valid);
    assert!(!got.has_dependency("c"));
    // A nested plain `json.Unmarshal` rejects duplicate names.
    assert!(!got.dev_dependencies.valid);
    assert!(!got.has_dependency("d"));
}
