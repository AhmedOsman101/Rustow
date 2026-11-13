//! Edge cases and failing cases for behavior rustow already supports.
//! Black-box through the CLI, tempdir fixtures only — real $HOME is never touched.

use std::fs;
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

// ---------- --no-stowrc / RUSTOW_NO_STOWRC -------------------------------

#[test]
fn no_stowrc_flag_disables_fallback_without_config() {
  let tmp = tempfile::tempdir().unwrap();
  let home = tmp.path().join("home");
  fs::create_dir_all(&home).unwrap();
  let stowrc = home.join(".stowrc");
  fs::write(&stowrc, "--dir=/nonexistent-stow-dir-for-test\n").unwrap();

  let mut cmd = Command::cargo_bin("rustow").unwrap();
  cmd
    .env("HOME", &home)
    .env("XDG_CONFIG_HOME", tmp.path().join("xdg"))
    .env_remove("DOTFILES_DIR")
    .env_remove("DOTFILES_TARGET")
    .env("NO_COLOR", "1")
    .args(["--no-stowrc", "link", "zsh"]);
  // stowrc would happily supply a (nonexistent) dir; with the flag, the run
  // must fail with the usual source-not-set guidance instead.
  cmd.assert().failure().stderr(contains("source"));
}

#[test]
fn no_stowrc_env_var_disables_fallback_without_config() {
  let tmp = tempfile::tempdir().unwrap();
  let home = tmp.path().join("home");
  fs::create_dir_all(&home).unwrap();
  let stowrc = home.join(".stowrc");
  fs::write(&stowrc, "--dir=/nonexistent-stow-dir-for-test\n").unwrap();

  let mut cmd = Command::cargo_bin("rustow").unwrap();
  cmd
    .env("HOME", &home)
    .env("XDG_CONFIG_HOME", tmp.path().join("xdg"))
    .env_remove("DOTFILES_DIR")
    .env_remove("DOTFILES_TARGET")
    .env("RUSTOW_NO_STOWRC", "1")
    .env("NO_COLOR", "1")
    .args(["link", "zsh"]);
  cmd.assert().failure().stderr(contains("source"));
}

#[test]
fn no_stowrc_flag_prints_a_note_when_stowrc_exists() {
  let tmp = tempfile::tempdir().unwrap();
  let home = tmp.path().join("home");
  fs::create_dir_all(&home).unwrap();
  let stowrc = home.join(".stowrc");
  fs::write(&stowrc, "--dir=/nonexistent-stow-dir-for-test\n").unwrap();

  let mut cmd = Command::cargo_bin("rustow").unwrap();
  cmd
    .env("HOME", &home)
    .env("XDG_CONFIG_HOME", tmp.path().join("xdg"))
    .env_remove("DOTFILES_DIR")
    .env_remove("DOTFILES_TARGET")
    .env("NO_COLOR", "1")
    .args(["--no-stowrc", "link", "zsh"]);
  cmd.assert().stderr(contains("ignoring"));
}

// ---------- non-ASCII ------------------------------------------------------

#[test]
fn non_ascii_package_and_paths_round_trip() {
  let r = rig();
  make_pkg(&r.source, "道具", &[".config/übersicht/widget.css"]);
  r.cmd().args(["link", "道具"]).assert().success();
  assert!(
    r.target.join(".config/übersicht/widget.css").exists()
      || r.target.join(".config/übersicht").exists()
  );
  r.cmd().args(["status", "道具"]).assert().success();
  r.cmd().args(["unlink", "道具"]).assert().success();
  assert!(!r.target.join(".config/übersicht").exists());
}

// ---------- fold = false ----------------------------------------------------

#[test]
fn fold_false_links_per_file_and_does_not_fold() {
  let r = rig();
  make_pkg(&r.source, "app", &[".config/app/settings.toml"]);
  fs::create_dir_all(r.xdg.join("rustow")).unwrap();
  fs::write(
    r.xdg.join("rustow/config.toml"),
    format!(
      "source = \"{}\"\ntarget = \"{}\"\nfold = false\n",
      r.source.display(),
      r.target.display()
    ),
  )
  .unwrap();
  let mut cmd = r.cmd();
  cmd.env_remove("DOTFILES_DIR").env_remove("DOTFILES_TARGET");
  cmd.args(["link", "app"]).assert().success();
  let meta = fs::symlink_metadata(r.target.join(".config/app")).unwrap();
  assert!(meta.is_dir() && !meta.file_type().is_symlink());
  assert!(
    fs::symlink_metadata(r.target.join(".config/app/settings.toml"))
      .unwrap()
      .file_type()
      .is_symlink()
  );
}

// ---------- unusual but valid input ----------------------------------------

#[test]
fn directory_named_zero_is_linked() {
  let r = rig();
  make_pkg(&r.source, "app", &[".config/app/0/file"]);
  r.cmd().args(["link", "app"]).assert().success();
  assert!(
    r.target.join(".config/app/0/file").exists() || r.target.join(".config/app").is_symlink()
  );
}

#[test]
fn unlink_of_never_linked_package_is_a_noop() {
  let r = rig();
  make_pkg(&r.source, "ghost", &[".config/ghost/cfg"]);
  r.cmd().args(["unlink", "ghost"]).assert().success();
  assert_eq!(fs::read_dir(&r.target).unwrap().count(), 0);
}

#[test]
fn link_and_unlink_of_empty_package_is_a_noop() {
  let r = rig();
  fs::create_dir_all(r.source.join("empty")).unwrap();
  r.cmd().args(["link", "empty"]).assert().success();
  assert_eq!(fs::read_dir(&r.target).unwrap().count(), 0);
  r.cmd().args(["unlink", "empty"]).assert().success();
}

// ---------- source == target ------------------------------------------------

#[test]
fn source_equal_target_is_refused() {
  let r = rig();
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  let mut cmd = r.cmd();
  cmd.env_remove("DOTFILES_DIR").env_remove("DOTFILES_TARGET");
  cmd.args([
    "--dir",
    r.source.to_str().unwrap(),
    "--target",
    r.source.to_str().unwrap(),
    "link",
    "zsh",
  ]);
  cmd.assert().failure().stderr(contains("same directory"));
  // nothing linked into the source tree: the package is exactly what we created
  assert!(!r.source.join("zsh/.zshrc").is_symlink());
}

// ---------- clean -----------------------------------------------------------

#[test]
fn clean_deletes_folded_link_whose_package_dir_was_removed() {
  let r = rig();
  make_pkg(&r.source, "eza", &[".config/eza/theme.yml"]);
  r.cmd().args(["link", "eza"]).assert().success();
  // `.config` doesn't exist in the target yet, so the fold lands on
  // `.config` itself — one symlink, not a per-file link.
  assert!(r.target.join(".config").is_symlink());
  // the whole package disappears from the source tree
  fs::remove_dir_all(r.source.join("eza")).unwrap();
  r.cmd().args(["clean"]).assert().success();
  assert!(!r.target.join(".config").is_symlink());
}

#[test]
fn clean_is_idempotent() {
  let r = rig();
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  r.cmd().args(["link", "zsh"]).assert().success();
  fs::remove_file(r.source.join("zsh/.zshrc")).unwrap();
  r.cmd().args(["clean"]).assert().success();
  r.cmd().args(["clean"]).assert().success();
  assert_eq!(fs::read_dir(&r.target).unwrap().count(), 0);
}
