//! CLI integration tests: tempdir fixtures, env isolation, assert_cmd.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use predicates::str::contains;

struct Rig {
  _tmp: tempfile::TempDir,
  source: PathBuf,
  target: PathBuf,
  xdg: PathBuf,
  home: PathBuf,
}

fn rig() -> Rig {
  let tmp = tempfile::tempdir().unwrap();
  let source = tmp.path().join("Configs");
  let target = tmp.path().join("home");
  let xdg = tmp.path().join("xdg");
  let home = tmp.path().join("userhome");
  fs::create_dir_all(&source).unwrap();
  fs::create_dir_all(&target).unwrap();
  fs::create_dir_all(&xdg).unwrap();
  fs::create_dir_all(&home).unwrap();
  Rig {
    _tmp: tmp,
    source,
    target,
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
      .env_remove("DOTFILES_DIR")
      .env_remove("DOTFILES_TARGET");
    cmd
  }

  fn cmd_env(&self) -> Command {
    let mut cmd = self.cmd();
    cmd
      .env("DOTFILES_DIR", &self.source)
      .env("DOTFILES_TARGET", &self.target);
    cmd
  }
}

fn make_pkg(source: &Path, pkg: &str, files: &[&str]) {
  let root = source.join(pkg);
  for f in files {
    let p = root.join(f);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, format!("contents of {f}")).unwrap();
  }
}

#[test]
fn no_args_fails() {
  let r = rig();
  r.cmd().assert().failure();
}

#[test]
fn link_no_args_fails() {
  let r = rig();
  r.cmd_env().args(["link"]).assert().failure();
}

#[test]
fn sync_links_all_packages() {
  let r = rig();
  make_pkg(&r.source, "eza", &[".config/eza/theme.yml"]);
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  r.cmd_env().args(["sync"]).assert().success();
  assert!(r.target.join(".zshrc").is_symlink());
  assert!(r.target.join(".config/eza/theme.yml").exists());
}

#[test]
fn link_missing_package_fails_cleanly() {
  let r = rig();
  r.cmd_env()
    .args(["link", "ghost"])
    .assert()
    .failure()
    .stderr(contains("ghost"));
}

#[test]
fn unlink_then_reset_removes_links() {
  let r = rig();
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  make_pkg(&r.source, "eza", &[".config/eza/theme.yml"]);
  r.cmd_env().args(["sync"]).assert().success();
  r.cmd_env().args(["unlink", "zsh"]).assert().success();
  assert!(!r.target.join(".zshrc").exists());
  assert!(r.target.join(".config/eza/theme.yml").exists());
  r.cmd_env().args(["reset"]).assert().success();
  assert!(!r.target.join(".config/eza").exists());
}

#[test]
fn remove_and_rm_are_aliases() {
  for alias in ["remove", "rm"] {
    let r = rig();
    make_pkg(&r.source, "zsh", &[".zshrc"]);
    r.cmd_env().args(["link", "zsh"]).assert().success();
    r.cmd_env().args([alias, "zsh"]).assert().success();
    assert!(!r.target.join(".zshrc").exists());
  }
}

#[test]
fn relink_refreshes() {
  let r = rig();
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  r.cmd_env().args(["link", "zsh"]).assert().success();
  r.cmd_env().args(["relink", "zsh"]).assert().success();
  assert!(r.target.join(".zshrc").is_symlink());
}

#[test]
fn dry_run_changes_nothing() {
  let r = rig();
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  r.cmd_env().args(["--dry-run", "sync"]).assert().success();
  assert!(!r.target.join(".zshrc").exists());
}

#[test]
fn conflict_aborts_and_reports_without_changes() {
  let r = rig();
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  fs::write(r.target.join(".zshrc"), "user contents").unwrap();
  r.cmd_env()
    .args(["link", "zsh"])
    .assert()
    .failure()
    .stderr(contains("conflict"));
  assert_eq!(
    fs::read_to_string(r.target.join(".zshrc")).unwrap(),
    "user contents"
  );
}

#[test]
fn force_overwrites_conflict() {
  let r = rig();
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  fs::write(r.target.join(".zshrc"), "user contents").unwrap();
  r.cmd_env().args(["-f", "link", "zsh"]).assert().success();
  assert!(r.target.join(".zshrc").is_symlink());
  assert_eq!(
    fs::read_to_string(r.target.join(".zshrc")).unwrap(),
    "contents of .zshrc"
  );
}

#[test]
fn env_only_invocation_warns_about_missing_config() {
  let r = rig();
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  r.cmd_env()
    .args(["link", "zsh"])
    .assert()
    .success()
    .stderr(contains("no config.toml"));
}

#[test]
fn config_toml_suppresses_warning() {
  let r = rig();
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  let cfg_dir = r.xdg.join("rustow");
  fs::create_dir_all(&cfg_dir).unwrap();
  fs::write(
    cfg_dir.join("config.toml"),
    format!(
      "source = {:?}\ntarget = {:?}\nverbose = false\n",
      r.source.to_string_lossy(),
      r.target.to_string_lossy()
    )
    .replace('{', "{ ")
    .replace('}', " }"),
  )
  .unwrap();
  // NOTE: the replace hack above is not needed with proper escaping; write plainly instead
  fs::write(
    cfg_dir.join("config.toml"),
    format!(
      "source = \"{}\"\ntarget = \"{}\"\nverbose = false\n",
      r.source.display(),
      r.target.display()
    ),
  )
  .unwrap();
  r.cmd()
    .args(["link", "zsh"])
    .assert()
    .success()
    .stderr(contains("no config.toml").not());
}

#[test]
fn source_not_set_errors_with_config_init_hint() {
  let r = rig();
  r.cmd()
    .args(["sync"])
    .assert()
    .failure()
    .stderr(contains("config init"));
}

#[test]
fn config_init_and_default() {
  let r = rig();
  let out = r.cmd().args(["config", "init"]).assert().success();
  let _ = out;
  let written = r.xdg.join("rustow/config.toml");
  assert!(written.exists());
  // second run refuses without --force
  r.cmd()
    .args(["config", "init"])
    .assert()
    .failure()
    .stderr(contains("already exists"));
  r.cmd()
    .args(["config", "init", "--force"])
    .assert()
    .success();
  // config default prints the annotated default
  let output = r.cmd().args(["config", "default"]).output().unwrap();
  let text = String::from_utf8(output.stdout).unwrap();
  assert!(text.contains("on_conflict"));
  assert!(text.contains("\".git\""));
}

#[test]
fn external_symlink_conflict_aborts() {
  let r = rig();
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  fs::write(r.target.join("foreign.txt"), "x").unwrap();
  symlink(r.target.join("foreign.txt"), r.target.join(".zshrc")).unwrap();
  r.cmd_env().args(["link", "zsh"]).assert().failure();
  assert!(r.target.join(".zshrc").is_symlink());
  assert_eq!(
    fs::read_link(r.target.join(".zshrc")).unwrap(),
    r.target.join("foreign.txt")
  );
}
