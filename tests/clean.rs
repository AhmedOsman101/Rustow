//! `rustow clean` tests: it must delete only rustow/stow-owned symlinks whose
//! target is missing, report-or-delete based on flags, and leave foreign links alone.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::str::contains;

struct Rig {
  _tmp: tempfile::TempDir,
  source: PathBuf,
  target: PathBuf,
  xdg: PathBuf,
}

fn rig() -> Rig {
  let tmp = tempfile::tempdir().unwrap();
  let source = tmp.path().join("Configs");
  let target = tmp.path().join("home");
  let xdg = tmp.path().join("xdg");
  fs::create_dir_all(&source).unwrap();
  fs::create_dir_all(&target).unwrap();
  fs::create_dir_all(&xdg).unwrap();
  Rig {
    _tmp: tmp,
    source,
    target,
    xdg,
  }
}

impl Rig {
  fn cmd(&self) -> Command {
    let mut cmd = Command::cargo_bin("rustow").unwrap();
    cmd
      .env("XDG_CONFIG_HOME", &self.xdg)
      .env("XDG_CACHE_HOME", &self.xdg)
      .env("DOTFILES_DIR", &self.source)
      .env("DOTFILES_TARGET", &self.target)
      .env("NO_COLOR", "1");
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
fn clean_reports_nothing_when_links_are_live() {
  let r = rig();
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  r.cmd().args(["link", "zsh"]).assert().success();
  r.cmd()
    .args(["clean"])
    .assert()
    .success()
    .stdout(contains("0"));
}

#[test]
fn clean_deletes_dangling_owned_link() {
  let r = rig();
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  r.cmd().args(["link", "zsh"]).assert().success();
  // simulate the package file disappearing → the symlink dangles
  fs::remove_file(r.source.join("zsh/.zshrc")).unwrap();
  r.cmd().args(["clean"]).assert().success();
  assert!(!r.target.join(".zshrc").exists() && !r.target.join(".zshrc").is_symlink());
}

#[test]
fn clean_leaves_foreign_dangling_symlink_alone() {
  let r = rig();
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  r.cmd().args(["link", "zsh"]).assert().success();
  fs::remove_file(r.source.join("zsh/.zshrc")).unwrap();
  // foreign dangling symlink (pointing outside the source tree)
  let foreign = r.target.join("foreign");
  symlink(r.target.join("does-not-exist"), &foreign).unwrap();
  r.cmd().args(["clean"]).assert().success();
  assert!(foreign.is_symlink());
}

#[test]
fn clean_dry_run_keeps_links_in_place() {
  let r = rig();
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  r.cmd().args(["link", "zsh"]).assert().success();
  fs::remove_file(r.source.join("zsh/.zshrc")).unwrap();
  r.cmd().args(["--dry-run", "clean"]).assert().success();
  assert!(r.target.join(".zshrc").is_symlink());
}
