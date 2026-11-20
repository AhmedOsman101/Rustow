//! `rustow config migrate` end-to-end tests (ADR 0004).
//!
//! Every fixture uses a tempdir with isolated HOME / XDG_* / DOTFILES_* so the
//! real ~/.stowrc and real dotfiles are never read or touched. The CLI is
//! driven through the real binary; the interactive prompt is a line-based
//! reader on stdin, the same seam `tests/exit_codes.rs` pins with `write_stdin`.

use std::fs;
use std::path::PathBuf;

use assert_cmd::Command;
use predicates::str::contains;

struct Rig {
  _tmp: tempfile::TempDir,
  source: PathBuf,
  other_source: PathBuf,
  target: PathBuf,
  xdg: PathBuf,
  cache: PathBuf,
  home: PathBuf,
  cfg_dir: PathBuf,
  cfg_path: PathBuf,
  stowrc: PathBuf,
}

fn rig() -> Rig {
  let tmp = tempfile::tempdir().unwrap();
  let source = tmp.path().join("Configs");
  let other_source = tmp.path().join("OtherConfigs");
  let target = tmp.path().join("home");
  let xdg = tmp.path().join("xdg-config");
  let cache = tmp.path().join("xdg-cache");
  let home = tmp.path().join("userhome");
  for dir in [&source, &other_source, &target, &xdg, &cache, &home] {
    fs::create_dir_all(dir).unwrap();
  }
  let cfg_dir = xdg.join("rustow");
  let cfg_path = cfg_dir.join("config.toml");
  let stowrc = home.join(".stowrc");
  Rig {
    _tmp: tmp,
    source,
    other_source,
    target,
    xdg,
    cache,
    home,
    cfg_dir,
    cfg_path,
    stowrc,
  }
}

impl Rig {
  fn cmd(&self) -> Command {
    let mut cmd = Command::cargo_bin("rustow").unwrap();
    cmd
      .env("HOME", &self.home)
      .env("XDG_CONFIG_HOME", &self.xdg)
      .env("XDG_CACHE_HOME", &self.cache)
      .env_remove("DOTFILES_DIR")
      .env_remove("DOTFILES_TARGET");
    cmd
  }

  fn write_stowrc(&self, contents: &str) {
    fs::write(&self.stowrc, contents).unwrap();
  }

  fn write_config(&self, contents: &str) {
    fs::create_dir_all(&self.cfg_dir).unwrap();
    fs::write(&self.cfg_path, contents).unwrap();
  }

  fn stowrc_contents(&self) -> String {
    format!(
      "--dir={}\n--target={}\n--verbose=0\n",
      self.source.display(),
      self.target.display()
    )
  }

  fn config_value(&self, key: &str) -> Option<toml::Value> {
    let text = fs::read_to_string(&self.cfg_path).unwrap();
    let parsed: toml::Value = toml::from_str(&text).unwrap();
    parsed.get(key).cloned()
  }
}

#[test]
fn adopt_from_empty_config_fills_all_gaps() {
  let r = rig();
  r.write_config("");
  r.write_stowrc(&r.stowrc_contents());

  r.cmd()
    .arg("config")
    .arg("migrate")
    .arg("--stowrc")
    .arg(&r.stowrc)
    .write_stdin("a\n")
    .assert()
    .success()
    .stdout(contains("source = "))
    .stdout(contains("target = "));

  assert_eq!(
    r.config_value("source")
      .and_then(|v| v.as_str().map(str::to_owned)),
    Some(r.source.to_string_lossy().into_owned())
  );
  assert_eq!(
    r.config_value("target")
      .and_then(|v| v.as_str().map(str::to_owned)),
    Some(r.target.to_string_lossy().into_owned())
  );
  assert_eq!(
    r.config_value("verbose").and_then(|v| v.as_bool()),
    Some(false)
  );
}

#[test]
fn existing_config_values_win_only_gaps_adopted() {
  let r = rig();
  r.write_config(&format!(
    "source = \"{}\"\nverbose = true\n",
    r.other_source.display()
  ));
  r.write_stowrc(&r.stowrc_contents());

  r.cmd()
    .arg("config")
    .arg("migrate")
    .arg("--stowrc")
    .arg(&r.stowrc)
    .arg("--yes")
    .assert()
    .success();

  // source + verbose were already set: config wins; target was the only gap.
  assert_eq!(
    r.config_value("source")
      .and_then(|v| v.as_str().map(str::to_owned)),
    Some(r.other_source.to_string_lossy().into_owned())
  );
  assert_eq!(
    r.config_value("verbose").and_then(|v| v.as_bool()),
    Some(true)
  );
  assert_eq!(
    r.config_value("target")
      .and_then(|v| v.as_str().map(str::to_owned)),
    Some(r.target.to_string_lossy().into_owned())
  );
}

#[test]
fn malformed_stowrc_errors_with_line_number_and_leaves_config_untouched() {
  let r = rig();
  r.write_config("verbose = true\n");
  r.write_stowrc("# preamble\n--dir=/tmp/x\n--target=\"unterminated\n");

  r.cmd()
    .arg("config")
    .arg("migrate")
    .arg("--stowrc")
    .arg(&r.stowrc)
    .arg("--yes")
    .assert()
    .failure()
    .stderr(contains("line 3"));

  assert_eq!(fs::read_to_string(&r.cfg_path).unwrap(), "verbose = true\n");
}

#[test]
fn adoption_never_deletes_or_renames_stowrc() {
  let r = rig();
  r.write_config("");
  let contents = r.stowrc_contents();
  r.write_stowrc(&contents);

  r.cmd()
    .arg("config")
    .arg("migrate")
    .arg("--stowrc")
    .arg(&r.stowrc)
    .arg("--yes")
    .assert()
    .success()
    .stdout(contains("left untouched"))
    .stdout(contains("remove"));

  assert_eq!(fs::read_to_string(&r.stowrc).unwrap(), contents);
  assert!(!r.home.join(".stowrc.bak").exists());
}

#[test]
fn dry_run_prints_plan_and_writes_nothing() {
  let r = rig();
  r.write_config("");
  r.write_stowrc(&r.stowrc_contents());
  let before = fs::read_to_string(&r.cfg_path).unwrap();

  r.cmd()
    .arg("config")
    .arg("migrate")
    .arg("--stowrc")
    .arg(&r.stowrc)
    .arg("--dry-run")
    .assert()
    .success()
    .stdout(contains("source = "))
    .stdout(contains("target = "))
    .stdout(contains("dry run"));

  assert_eq!(fs::read_to_string(&r.cfg_path).unwrap(), before);
}

#[test]
fn yes_flag_skips_the_prompt() {
  let r = rig();
  r.write_config("");
  r.write_stowrc(&r.stowrc_contents());

  // EOF on stdin: if the prompt ran, it would skip and adopt nothing. Adoption
  // still happening proves --yes skipped the prompt entirely.
  r.cmd()
    .arg("config")
    .arg("migrate")
    .arg("--stowrc")
    .arg(&r.stowrc)
    .arg("--yes")
    .write_stdin("")
    .assert()
    .success();

  assert!(
    r.config_value("source")
      .and_then(|v| v.as_str().map(str::to_owned))
      .is_some()
  );
}

#[test]
fn stowrc_pointing_at_nonexistent_dir_errors_and_writes_nothing() {
  let r = rig();
  let missing = r.home.join("moved-away");
  r.write_config("");
  r.write_stowrc(&format!(
    "--dir={}\n--target={}\n",
    missing.display(),
    r.target.display()
  ));

  r.cmd()
    .arg("config")
    .arg("migrate")
    .arg("--stowrc")
    .arg(&r.stowrc)
    .arg("--yes")
    .assert()
    .failure()
    .stderr(contains("does not exist"));

  assert_eq!(fs::read_to_string(&r.cfg_path).unwrap(), "");
}

#[test]
fn interactive_skip_leaves_config_untouched() {
  let r = rig();
  r.write_config("verbose = true\n");
  r.write_stowrc(&r.stowrc_contents());

  r.cmd()
    .arg("config")
    .arg("migrate")
    .arg("--stowrc")
    .arg(&r.stowrc)
    .write_stdin("s\n")
    .assert()
    .success()
    .stdout(contains("skipped"));

  assert_eq!(fs::read_to_string(&r.cfg_path).unwrap(), "verbose = true\n");
}

#[test]
fn adoption_creates_config_when_none_exists() {
  let r = rig();
  assert!(!r.cfg_path.exists());
  r.write_stowrc(&r.stowrc_contents());

  r.cmd()
    .arg("config")
    .arg("migrate")
    .arg("--stowrc")
    .arg(&r.stowrc)
    .arg("--yes")
    .assert()
    .success();

  assert!(r.cfg_path.exists());
  assert!(
    r.config_value("source")
      .and_then(|v| v.as_str().map(str::to_owned))
      .is_some()
  );
}

#[test]
fn shared_migrate_stowrc_api_returns_only_gaps() {
  // Pins the reusable seam Task 22 (config init) will call.
  let r = rig();
  r.write_config(&format!("source = \"{}\"\n", r.other_source.display()));
  r.write_stowrc(&r.stowrc_contents());

  let plan = rustow::config::migrate_stowrc(&r.cfg_path, &r.stowrc).unwrap();
  assert!(plan.config_existed);
  assert_eq!(plan.stowrc_path, r.stowrc);
  let keys: Vec<&str> = plan.additions.iter().map(|a| a.key).collect();
  assert_eq!(keys, vec!["target", "verbose"]); // source already set -> wins

  let written = plan.apply().unwrap();
  assert_eq!(written, r.cfg_path);
  assert_eq!(
    r.config_value("source")
      .and_then(|v| v.as_str().map(str::to_owned)),
    Some(r.other_source.to_string_lossy().into_owned())
  );
}
