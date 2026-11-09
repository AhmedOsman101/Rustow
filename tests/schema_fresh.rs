//! schema.json must be regenerated via `mask schema` when Config changes.
#[test]
fn schema_json_is_fresh() {
    let schema = schemars::schema_for!(rustow::config::Config);
    let expected = serde_json::to_string_pretty(&schema).unwrap() + "\n";
    let current = std::fs::read_to_string("schema.json").unwrap();
    assert_eq!(expected, current, "schema.json is stale — run `mask schema`");
}

#[test]
fn docs_config_toml_matches_default() {
    let expected = rustow::config::print_default();
    let current = std::fs::read_to_string("docs/config.toml").unwrap();
    assert_eq!(expected, current, "docs/config.toml drifted from print_default()");
}
