//! Task 17: config schema for `[package.<name>.root]` (ADR 0001, amended
//! 2026-11-20).
//!
//! Config load validates SHAPE only: keys stay inside the package, values are
//! absolute after `~` expansion, and trailing slashes are cosmetic. The mapping
//! kind (file vs directory) follows the source path's actual filesystem type at
//! plan time — Task 18 — so a missing source path is NOT a config-load error.
//!
//! Every fixture isolates HOME/XDG_*/DOTFILES_*; the real machine is untouched.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use rustow::config::{Config, normalize_root_mapping};

fn parse(body: &str) -> Config {
  toml::from_str(body).expect("config must parse")
}

fn root_of(cfg: &Config, pkg: &str) -> BTreeMap<String, String> {
  cfg
    .package
    .get(pkg)
    .expect("package table must be present")
    .root
    .clone()
    .expect("root mapping must be present")
}

/// (Step 1) A file-style key (`configs/root`) and a directory-style key
/// (`configs`) both parse. Which kind each one is, is decided later from the
/// source path's filesystem type — not here.
#[test]
fn parses_file_and_dir_mappings() {
  let cfg = parse(
    "[package.snapper.root]\n\
     \"configs/root\" = \"/etc/snapper/root\"\n\
     \"configs\" = \"/etc/snapper\"\n",
  );
  let root = root_of(&cfg, "snapper");
  assert_eq!(
    root.get("configs/root").map(String::as_str),
    Some("/etc/snapper/root")
  );
  assert_eq!(
    root.get("configs").map(String::as_str),
    Some("/etc/snapper")
  );
  assert_eq!(root.len(), 2);
}

/// (Step 1) A `..` escape in a key is rejected with an error naming the key —
/// the same rule `create --path` uses to keep paths inside the package.
#[test]
fn root_key_dotdot_escape_is_config_error() {
  let err = toml::from_str::<Config>("[package.snapper.root]\n\"../etc\" = \"/etc\"\n")
    .expect_err("an escaping key must be rejected at load");
  let msg = err.to_string();
  assert!(
    msg.contains(".."),
    "error must name the offending key, got: {msg}"
  );
  assert!(
    msg.contains("escape"),
    "error must explain the escape, got: {msg}"
  );
}

/// (Step 1) An absolute key escapes the package too (same as `create --path`).
#[test]
fn root_key_absolute_is_config_error() {
  let err = toml::from_str::<Config>("[package.snapper.root]\n\"/etc\" = \"/etc\"\n")
    .expect_err("an absolute key must be rejected at load");
  assert!(
    err.to_string().contains("escape"),
    "error must explain the escape, got: {err}"
  );
}

/// (Step 1) A `.`-only key has no real component and is rejected.
#[test]
fn root_key_without_component_is_config_error() {
  let err = toml::from_str::<Config>("[package.snapper.root]\n\".\" = \"/etc\"\n")
    .expect_err("a '.' key must be rejected at load");
  assert!(
    err.to_string().contains("component"),
    "error must explain the missing component, got: {err}"
  );
}

/// (Step 1) A non-absolute value is rejected and the error names the mapping
/// key so a human can find the bad line.
#[test]
fn non_absolute_value_errors_naming_the_key() {
  let err = toml::from_str::<Config>("[package.snapper.root]\n\"configs\" = \"etc/snapper\"\n")
    .expect_err("a relative destination must be rejected");
  let msg = err.to_string();
  assert!(
    msg.contains("configs"),
    "error must name the mapping key, got: {msg}"
  );
  assert!(
    msg.contains("absolute"),
    "error must explain absoluteness, got: {msg}"
  );
}

/// (Step 1) A leading `~` expands to the supplied HOME and the result is an
/// absolute, normalized string. The helper is pure (HOME is passed in), so
/// this never reads the real machine's HOME.
#[test]
fn tilde_value_expands_to_home() {
  let home = Path::new("/tmp/opencode/rustow-task17-fake-home");
  let (key, value) = normalize_root_mapping("configs/", "~/snapper/", home).unwrap();
  assert_eq!(key, "configs");
  assert_eq!(value, "/tmp/opencode/rustow-task17-fake-home/snapper");

  // A bare `~` is the home directory itself.
  let (_, value) = normalize_root_mapping("cfg", "~", home).unwrap();
  assert_eq!(value, "/tmp/opencode/rustow-task17-fake-home");

  // `~user` is not a supported expansion: it stays relative and must fail.
  let err = normalize_root_mapping("cfg", "~someone/cfg", home).unwrap_err();
  assert!(err.to_string().contains("absolute"), "got: {err}");
}

/// (Step 1) Trailing slashes on a key or a value are cosmetic; all four spellings
/// produce the identical normalized map.
#[test]
fn trailing_slashes_are_cosmetic() {
  let plain = root_of(
    &parse("[package.snapper.root]\n\"configs\" = \"/etc/snapper\"\n"),
    "snapper",
  );
  let both = root_of(
    &parse("[package.snapper.root]\n\"configs/\" = \"/etc/snapper/\"\n"),
    "snapper",
  );
  let key_only = root_of(
    &parse("[package.snapper.root]\n\"configs/\" = \"/etc/snapper\"\n"),
    "snapper",
  );
  let value_only = root_of(
    &parse("[package.snapper.root]\n\"configs\" = \"/etc/snapper/\"\n"),
    "snapper",
  );
  assert_eq!(plain, both);
  assert_eq!(plain, key_only);
  assert_eq!(plain, value_only);
  assert_eq!(
    plain.get("configs").map(String::as_str),
    Some("/etc/snapper")
  );
}

/// (Step 1, amendment) Config load cannot know the source path's filesystem
/// type, so a mapped source that does not exist on disk still parses here;
/// rejecting it is plan-time work (Task 18).
#[test]
fn missing_source_path_is_not_a_config_error() {
  let cfg = parse("[package.snapper.root]\n\"configs/definitely-absent\" = \"/etc/snapper/x\"\n");
  assert!(
    root_of(&cfg, "snapper").contains_key("configs/definitely-absent"),
    "shape-only config load must accept a not-yet-resolved source path"
  );
}

/// `root` must be a table; a scalar is a config error that names the key.
#[test]
fn root_must_be_a_table() {
  let err = toml::from_str::<Config>("[package.snapper]\nroot = \"/etc\"\n")
    .expect_err("root must be a table");
  assert!(
    err.to_string().contains("root"),
    "error must point at the root key, got: {err}"
  );
}

/// Adding the `root` field must not weaken `deny_unknown_fields`.
#[test]
fn unknown_key_in_package_table_is_still_an_error() {
  let err = toml::from_str::<Config>("[package.snapper]\nbogus = 1\n")
    .expect_err("deny_unknown_fields must be preserved inside [package.x]");
  assert!(
    err.to_string().contains("bogus"),
    "error must name the unknown key, got: {err}"
  );
}

// --- CLI surface with fully isolated environment -------------------------

struct Rig {
  _tmp: tempfile::TempDir,
  source: PathBuf,
  xdg: PathBuf,
  home: PathBuf,
}

fn rig(config_body: &str) -> Rig {
  let base = Path::new("/tmp/opencode");
  let mut builder = tempfile::Builder::new();
  builder.prefix("rustow-task17-");
  let tmp = if base.is_dir() {
    builder.tempdir_in(base).unwrap()
  } else {
    builder.tempdir().unwrap()
  };

  let source = tmp.path().join("Configs");
  let xdg = tmp.path().join("xdg");
  let home = tmp.path().join("home");
  fs::create_dir_all(&source).unwrap();
  fs::create_dir_all(xdg.join("rustow")).unwrap();
  fs::create_dir_all(&home).unwrap();
  fs::write(xdg.join("rustow/config.toml"), config_body).unwrap();

  Rig {
    _tmp: tmp,
    source,
    xdg,
    home,
  }
}

impl Rig {
  fn cmd(&self) -> Command {
    let mut cmd = Command::cargo_bin("rustow").unwrap();
    cmd
      .env("XDG_CONFIG_HOME", &self.xdg)
      .env("XDG_CACHE_HOME", &self.xdg)
      .env("HOME", &self.home)
      .env("DOTFILES_DIR", &self.source)
      .env("DOTFILES_TARGET", self.home.join("target"))
      .env("NO_COLOR", "1");
    cmd
  }

  fn make_pkg(&self, pkg: &str, files: &[&str]) {
    let root = self.source.join(pkg);
    for f in files {
      let p = root.join(f);
      fs::create_dir_all(p.parent().unwrap()).unwrap();
      fs::write(p, format!("contents of {f}")).unwrap();
    }
  }
}

/// The `..` config error surfaces through the real binary.
#[test]
fn dotdot_key_surfaces_at_cli() {
  let r = rig("[package.snapper.root]\n\"../etc\" = \"/etc\"\n");
  let out = r.cmd().args(["status", "snapper"]).assert().failure();
  let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
  assert!(
    stderr.contains("..") && stderr.contains("escape"),
    "CLI error must name the escaping key, got: {stderr}"
  );
}

/// A `~` destination expands to an absolute path under the isolated HOME, so
/// config load succeeds. (Without `~` the same string would be relative and
/// rejected — pinned by `non_absolute_value_errors_naming_the_key`.)
#[test]
fn tilde_value_parses_under_isolated_home() {
  let r = rig("[package.snapper.root]\n\"configs\" = \"~/snapper\"\n");
  r.make_pkg("snapper", &["existing.conf"]);
  r.cmd()
    .args(["link", "--dry-run", "snapper"])
    .assert()
    .success();
}
