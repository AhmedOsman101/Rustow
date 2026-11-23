//! schema.json must be regenerated via `mask schema` when Config changes.

const SCHEMA_URL: &str =
  "https://raw.githubusercontent.com/AhmedOsman101/Rustow/refs/heads/main/schema.json";

#[test]
fn default_config_declares_schema_and_parses() {
  let default = rustow::config::print_default();
  assert!(
    default.contains(&format!("\"$schema\" = \"{SCHEMA_URL}\"")),
    "default config is missing the $schema key"
  );
  let parsed: rustow::config::Config = toml::from_str(&default).unwrap();
  assert_eq!(parsed.schema.as_deref(), Some(SCHEMA_URL));
}

#[test]
fn schema_exposes_the_config_schema_key() {
  let current: serde_json::Value =
    serde_json::from_str(&std::fs::read_to_string("schema.json").unwrap()).unwrap();
  assert_eq!(current["properties"]["$schema"]["default"], SCHEMA_URL);
}

#[test]
fn schema_json_is_fresh() {
  let schema = schemars::schema_for!(rustow::config::Config);
  let expected = serde_json::to_value(&schema).unwrap();
  let current: serde_json::Value =
    serde_json::from_str(&std::fs::read_to_string("schema.json").unwrap()).unwrap();
  assert_eq!(
    current, expected,
    "schema.json is stale — run `mask schema`"
  );
}

#[test]
fn docs_config_toml_matches_default() {
  let expected = rustow::config::print_default();
  let current = std::fs::read_to_string("docs/config.toml").unwrap();
  assert_eq!(
    expected, current,
    "docs/config.toml drifted from print_default()"
  );
}
