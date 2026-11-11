use std::{env, fs, path::Path};

fn first_diff(a: &serde_json::Value, b: &serde_json::Value) -> Option<String> {
  match (a, b) {
    (serde_json::Value::Object(x), serde_json::Value::Object(y)) => {
      let mut keys: Vec<&str> = x.keys().chain(y.keys()).map(String::as_str).collect();
      keys.sort();
      keys.dedup();
      for k in keys {
        match (x.get(k), y.get(k)) {
          (Some(xv), Some(yv)) => {
            if let Some(d) = first_diff(xv, yv) {
              return Some(format!("{k}.{d}"));
            }
          }
          (Some(_), None) => return Some(format!("extra key `{k}` in schema.json")),
          (None, Some(_)) => return Some(format!("missing key `{k}` in schema.json")),
        }
      }
      None
    }
    (serde_json::Value::Array(x), serde_json::Value::Array(y)) => {
      if x.len() != y.len() {
        return Some(format!("array length {} != {}", x.len(), y.len()));
      }
      for (i, (xv, yv)) in x.iter().zip(y.iter()).enumerate() {
        if let Some(d) = first_diff(xv, yv) {
          return Some(format!("[{i}].{d}"));
        }
      }
      None
    }
    _ => (a != b).then(|| format!("{a:?} != {b:?}")),
  }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
  let schema = schemars::schema_for!(rustow::config::Config);
  let json = serde_json::to_string_pretty(&schema)? + "\n";
  let path = Path::new("schema.json");
  if env::args().any(|a| a == "--check") {
    let current: serde_json::Value = fs::read_to_string(path)
      .map(|s| serde_json::from_str::<serde_json::Value>(&s))
      .and_then(|r| r.map_err(std::io::Error::other))
      .unwrap_or_else(|_| serde_json::json!({}));
    let fresh: serde_json::Value = serde_json::from_str(&json)?;
    if current != fresh {
      match first_diff(&current, &fresh) {
        Some(d) => eprintln!("schema.json is stale at `{d}` — run `mask schema`"),
        None => eprintln!("schema.json is stale — run `mask schema`"),
      }
      std::process::exit(1);
    }
    return Ok(());
  }
  fs::write(path, json)?;
  Ok(())
}
