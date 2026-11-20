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

#[test]
fn missing_target_directory_is_adopted_with_a_warning() {
  // A target that does not exist yet is fine (created on link); only the
  // source must exist. The migration succeeds and records a warning.
  let r = rig();
  let missing_target = r.home.join("target-to-be-created");
  assert!(!missing_target.exists());
  r.write_config("");
  r.write_stowrc(&format!(
    "--dir={}\n--target={}\n--verbose=1\n",
    r.source.display(),
    missing_target.display()
  ));

  r.cmd()
    .arg("config")
    .arg("migrate")
    .arg("--stowrc")
    .arg(&r.stowrc)
    .arg("--yes")
    .assert()
    .success()
    .stderr(contains("does not exist yet"));

  assert_eq!(
    r.config_value("target")
      .and_then(|v| v.as_str().map(str::to_owned)),
    Some(missing_target.to_string_lossy().into_owned())
  );
}

#[test]
fn malformed_stowrc_missing_value_errors_and_writes_nothing() {
  let r = rig();
  r.write_config("verbose = true\n");
  r.write_stowrc("# preamble\n--dir\n");

  r.cmd()
    .arg("config")
    .arg("migrate")
    .arg("--stowrc")
    .arg(&r.stowrc)
    .arg("--yes")
    .assert()
    .failure()
    .stderr(contains("line 2"))
    .stderr(contains(".stowrc"));

  assert_eq!(fs::read_to_string(&r.cfg_path).unwrap(), "verbose = true\n");
}

// --- Task 22: the same adoption offer inline in `config init` (ADR 0004) ----

#[test]
fn init_with_stowrc_shows_adoption_prompt() {
  let r = rig();
  r.write_stowrc(&r.stowrc_contents());

  r.cmd()
    .args(["config", "init"])
    .write_stdin("k\n")
    .assert()
    .success()
    .stdout(contains("stowrc -> config.toml"))
    .stdout(contains("[a]dopt"))
    .stdout(contains("[k]eep defaults"));

  assert!(r.cfg_path.exists());
}

#[test]
fn init_adopt_merges_stowrc_values_into_default() {
  let r = rig();
  let contents = r.stowrc_contents();
  r.write_stowrc(&contents);

  r.cmd()
    .args(["config", "init"])
    .write_stdin("a\n")
    .assert()
    .success();

  // Adoption keeps the annotated default and adds the stowrc values on top.
  let text = fs::read_to_string(&r.cfg_path).unwrap();
  assert!(
    text.contains("# rustow — default configuration"),
    "adopted config should still be the annotated default: {text}"
  );
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
  // The stowrc is an input to the migration, never something it removes.
  assert_eq!(fs::read_to_string(&r.stowrc).unwrap(), contents);
  assert!(!r.home.join(".stowrc.bak").exists());
}

#[test]
fn init_keep_writes_documented_default() {
  let r = rig();
  r.write_stowrc(&r.stowrc_contents());

  r.cmd()
    .args(["config", "init"])
    .write_stdin("k\n")
    .assert()
    .success()
    .stdout(contains("skipped"));

  assert_eq!(
    fs::read_to_string(&r.cfg_path).unwrap(),
    rustow::config::print_default()
  );
}

#[test]
fn init_yes_adopts_non_interactively() {
  let r = rig();
  r.write_stowrc(&r.stowrc_contents());

  // EOF on stdin: if the prompt ran, it would keep defaults and adopt nothing.
  // Adoption still happening proves --yes skipped the prompt.
  r.cmd()
    .args(["config", "init", "--yes"])
    .write_stdin("")
    .assert()
    .success()
    .stdout(contains("stowrc -> config.toml"));

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
}

#[test]
fn init_without_stowrc_writes_default_without_prompt() {
  let r = rig();
  assert!(!r.stowrc.exists());

  let out = r
    .cmd()
    .args(["config", "init"])
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();
  let stdout = String::from_utf8(out).unwrap();
  assert!(!stdout.contains("[a]dopt"), "unexpected prompt: {stdout}");

  assert_eq!(
    fs::read_to_string(&r.cfg_path).unwrap(),
    rustow::config::print_default()
  );
}

#[test]
fn init_force_semantics_unchanged_when_stowrc_is_absent() {
  let r = rig();
  r.write_config("verbose = true\n");

  r.cmd()
    .args(["config", "init"])
    .assert()
    .failure()
    .stderr(contains("already exists"));
  r.cmd()
    .args(["config", "init", "--force"])
    .assert()
    .success();

  assert_eq!(
    fs::read_to_string(&r.cfg_path).unwrap(),
    rustow::config::print_default()
  );
}

#[test]
fn init_force_adoption_uses_stowrc_values_not_existing_config() {
  // --force replaces the config wholesale, so the old file's keys must not
  // suppress adoption of the stowrc values (review F1).
  let r = rig();
  r.write_config("source = \"/keep\"\n");
  r.write_stowrc(&r.stowrc_contents());

  r.cmd()
    .args(["config", "init", "--force", "--yes"])
    .write_stdin("")
    .assert()
    .success()
    .stdout(contains("stowrc -> config.toml"));

  let text = fs::read_to_string(&r.cfg_path).unwrap();
  assert!(
    !text.contains("/keep"),
    "old value should be replaced: {text}"
  );
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
fn init_no_stowrc_flag_skips_adoption_offer() {
  // The shipped kill-switch must silence the automatic init offer too (F2).
  let r = rig();
  r.write_stowrc(&r.stowrc_contents());

  let out = r
    .cmd()
    .args(["config", "init", "--no-stowrc"])
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();
  let stdout = String::from_utf8(out).unwrap();
  assert!(!stdout.contains("[a]dopt"), "unexpected offer: {stdout}");
  assert_eq!(
    fs::read_to_string(&r.cfg_path).unwrap(),
    rustow::config::print_default()
  );
}

#[test]
fn init_no_stowrc_env_skips_adoption_offer() {
  let r = rig();
  r.write_stowrc(&r.stowrc_contents());

  let out = r
    .cmd()
    .args(["config", "init"])
    .env("RUSTOW_NO_STOWRC", "1")
    .assert()
    .success()
    .get_output()
    .stdout
    .clone();
  let stdout = String::from_utf8(out).unwrap();
  assert!(!stdout.contains("[a]dopt"), "unexpected offer: {stdout}");
  assert_eq!(
    fs::read_to_string(&r.cfg_path).unwrap(),
    rustow::config::print_default()
  );
}
