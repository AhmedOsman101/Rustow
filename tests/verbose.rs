//! Verbose output format pins for link/unlink/clean (§5.2.3).
//!
//! Every fixture lives in a tempdir with isolated HOME, XDG_* and DOTFILES_*
//! so no command can read or mutate the live machine's dotfiles.

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;

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
      .env("HOME", &self.home)
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

fn stdout_lines(out: &assert_cmd::assert::Assert) -> Vec<String> {
  String::from_utf8(out.get_output().stdout.clone())
    .unwrap()
    .lines()
    .map(str::to_owned)
    .collect()
}

#[test]
fn link_verbose_prints_one_link_line_per_action() {
  let r = rig();
  // Two top-level files => two Link actions (no folding at the package root).
  make_pkg(&r.source, "app", &[".alpha", ".beta"]);

  let out = r.cmd().args(["link", "-v", "app"]).assert().success();
  let lines = stdout_lines(&out);

  let link_lines: Vec<&String> = lines.iter().filter(|l| l.starts_with("link  ")).collect();
  assert_eq!(link_lines.len(), 2, "stdout: {lines:?}");

  for file in [".alpha", ".beta"] {
    let needle = format!(
      "{} -> {}",
      r.target.join(file).display(),
      r.source.join("app").join(file).display()
    );
    assert!(
      link_lines.iter().any(|l| l.contains(&needle)),
      "expected a 'link  <dst> -> <src>' line containing {needle:?}; stdout: {lines:?}"
    );
  }
}

#[test]
fn unlink_verbose_prints_one_line_per_unlinked_link() {
  let r = rig();
  make_pkg(&r.source, "app", &[".alpha", ".beta"]);
  r.cmd().args(["link", "app"]).assert().success();

  let out = r.cmd().args(["unlink", "-v", "app"]).assert().success();
  let lines = stdout_lines(&out);

  let unlink_lines: Vec<&String> = lines.iter().filter(|l| l.starts_with("unlink ")).collect();
  assert_eq!(unlink_lines.len(), 2, "stdout: {lines:?}");
  for file in [".alpha", ".beta"] {
    let dst = r.target.join(file).display().to_string();
    assert!(
      unlink_lines.iter().any(|l| l.contains(&dst)),
      "expected an 'unlink <dst>' line containing {dst:?}; stdout: {lines:?}"
    );
  }
}

#[test]
fn clean_verbose_prints_each_broken_link() {
  let r = rig();
  make_pkg(&r.source, "app", &[".alpha", ".beta"]);
  r.cmd().args(["link", "app"]).assert().success();
  // Dangle both owned links by removing their sources.
  fs::remove_file(r.source.join("app/.alpha")).unwrap();
  fs::remove_file(r.source.join("app/.beta")).unwrap();

  let out = r.cmd().args(["clean", "-v"]).assert().success();
  let lines = stdout_lines(&out);

  // The trailing summary ("deleted N broken link(s)") shares the verb prefix,
  // so select only the per-link lines by requiring the target path.
  let target = r.target.display().to_string();
  let deleted_lines: Vec<&String> = lines
    .iter()
    .filter(|l| l.starts_with("deleted ") && l.contains(&target))
    .collect();
  assert_eq!(deleted_lines.len(), 2, "stdout: {lines:?}");
  for file in [".alpha", ".beta"] {
    let dst = r.target.join(file).display().to_string();
    assert!(
      deleted_lines.iter().any(|l| l.contains(&dst)),
      "expected a 'deleted <dst>' line containing {dst:?}; stdout: {lines:?}"
    );
  }
}
