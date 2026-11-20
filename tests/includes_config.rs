//! Task 15: config schema for `[package.<name>] includes` (ADR 0002).
//!
//! These tests exercise the public config seam: the `Config` model parses
//! per-package tables, and malformed input fails with an error that names the
//! offending package key.

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use rustow::config::Config;

#[test]
fn parses_valid_per_package_includes() {
  let cfg: Config = toml::from_str("[package.eza]\nincludes = [\"**\", \"!**/*.local\"]\n")
    .expect("a biome-style includes list must parse");

  let eza = cfg.package.get("eza").expect("package.eza must be present");
  assert_eq!(
    eza.includes,
    Some(vec!["**".to_string(), "!**/*.local".to_string()])
  );
}

#[test]
fn malformed_include_glob_errors_naming_the_package_key() {
  let err = toml::from_str::<Config>("[package.eza]\nincludes = [\"[unclosed\"]\n")
    .expect_err("a malformed glob must be rejected at load");
  assert!(
    err.to_string().contains("package \"eza\""),
    "error must name the package key, got: {err}"
  );
}

#[test]
fn wrong_type_includes_is_an_error() {
  let err = toml::from_str::<Config>("[package.eza]\nincludes = { pattern = \"**\" }\n")
    .expect_err("a table is not a valid includes value");
  let msg = err.to_string();
  assert!(
    msg.contains("includes"),
    "error must point at the includes key, got: {msg}"
  );
}

#[test]
fn unknown_key_in_package_table_is_an_error() {
  let err = toml::from_str::<Config>("[package.eza]\nbogus = 1\n")
    .expect_err("deny_unknown_fields must be preserved inside [package.x]");
  let msg = err.to_string();
  assert!(
    msg.contains("bogus"),
    "error must name the unknown key, got: {msg}"
  );
}

#[test]
fn malformed_include_glob_surfaces_at_cli() {
  // Isolated fixture: no real HOME/XDG/DOTFILES_* is ever touched.
  let base = Path::new("/tmp/opencode");
  let mut builder = tempfile::Builder::new();
  builder.prefix("rustow-task15-");
  let tmp = if base.is_dir() {
    builder.tempdir_in(base).unwrap()
  } else {
    builder.tempdir().unwrap()
  };
  let xdg = tmp.path().join("xdg");
  let home = tmp.path().join("home");
  let src = tmp.path().join("src");
  fs::create_dir_all(xdg.join("rustow")).unwrap();
  fs::create_dir_all(&home).unwrap();
  fs::create_dir_all(&src).unwrap();
  fs::write(
    xdg.join("rustow/config.toml"),
    "[package.eza]\nincludes = [\"[unclosed\"]\n",
  )
  .unwrap();

  let out = Command::cargo_bin("rustow")
    .unwrap()
    .env("XDG_CONFIG_HOME", &xdg)
    .env("XDG_CACHE_HOME", &xdg)
    .env("HOME", &home)
    .env_remove("DOTFILES_DIR")
    .env_remove("DOTFILES_TARGET")
    .args([
      "status",
      "--dir",
      src.to_str().unwrap(),
      "--target",
      home.to_str().unwrap(),
    ])
    .output()
    .unwrap();

  assert!(
    !out.status.success(),
    "CLI must fail on a malformed includes glob"
  );
  let stderr = String::from_utf8(out.stderr).unwrap();
  assert!(
    stderr.contains("package \"eza\""),
    "CLI error must name the package key, got: {stderr}"
  );
}
