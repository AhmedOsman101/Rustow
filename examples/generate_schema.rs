use std::{env, fs, path::Path};

fn main() -> Result<(), Box<dyn std::error::Error>> {
  let schema = schemars::schema_for!(rustow::config::Config);
  let json = serde_json::to_string_pretty(&schema)? + "\n";
  let path = Path::new("schema.json");
  if env::args().any(|a| a == "--check") {
    let current = fs::read_to_string(path).unwrap_or_default();
    if current != json {
      eprintln!("schema.json is stale — run `mask schema`");
      std::process::exit(1);
    }
    return Ok(());
  }
  fs::write(path, json)?;
  Ok(())
}
