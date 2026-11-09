//! `rustow create` integration tests (non-interactive paths).

use std::fs;
use std::path::Path;

use assert_cmd::Command;
use predicates::str::contains;

struct Rig {
  _tmp: tempfile::TempDir,
  source: std::path::PathBuf,
  target: std::path::PathBuf,
  xdg: std::path::PathBuf,
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

fn write_file(root: &Path, rel: &str, contents: &str) {
  let p = root.join(rel);
  fs::create_dir_all(p.parent().unwrap()).unwrap();
  fs::write(p, contents).unwrap();
}

#[test]
fn create_empty_package() {
  let r = rig();
  r.cmd()
    .args(["create", "vim", "--empty"])
    .assert()
    .success();
  let pkg = r.source.join("vim");
  assert!(pkg.is_dir());
  // nothing else created inside the package — empty means empty
  assert_eq!(fs::read_dir(&pkg).unwrap().count(), 0);
  assert!(!pkg.join(".config").exists());
}

#[test]
fn create_add_is_an_alias() {
  let r = rig();
  r.cmd().args(["add", "vim", "--empty"]).assert().success();
  assert!(r.source.join("vim").is_dir());
}

#[test]
fn create_moves_existing_source_and_links() {
  let r = rig();
  write_file(&r.target, ".config/vim/vimrc", "set nocompatible");
  r.cmd().args(["create", "vim", "--yes"]).assert().success();
  let pkg = r.source.join("vim");
  assert_eq!(
    fs::read_to_string(pkg.join(".config/vim/vimrc")).unwrap(),
    "set nocompatible"
  );
  assert!(!r.target.join(".config/vim").is_dir() || r.target.join(".config/vim").is_symlink());
  // linked: target path resolves to package contents
  assert_eq!(
    fs::read_to_string(r.target.join(".config/vim/vimrc")).unwrap(),
    "set nocompatible"
  );
}

#[test]
fn create_existing_source_requires_yes_non_interactive() {
  let r = rig();
  write_file(&r.target, ".config/vim/vimrc", "set nocompatible");
  r.cmd().args(["create", "vim"]).assert().failure();
  // nothing was moved
  assert_eq!(
    fs::read_to_string(r.target.join(".config/vim/vimrc")).unwrap(),
    "set nocompatible"
  );
  assert!(!r.source.join("vim").exists());
}

#[test]
fn create_missing_source_errors_with_guidance() {
  let r = rig();
  let out = r.cmd().args(["create", "vim"]).assert().failure();
  let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
  assert!(stderr.contains("--empty"), "stderr: {stderr}");
}

#[test]
fn create_source_flag_moves_from_custom_path() {
  let r = rig();
  let other = r.target.parent().unwrap().join("elsewhere/dotvimrc");
  fs::create_dir_all(other.parent().unwrap()).unwrap();
  fs::write(&other, "custom vimrc").unwrap();
  r.cmd()
    .args([
      "create",
      "vim",
      "--yes",
      "--source",
      other.to_str().unwrap(),
      "--path",
      ".vimrc",
    ])
    .assert()
    .success();
  assert_eq!(
    fs::read_to_string(r.source.join("vim/.vimrc")).unwrap(),
    "custom vimrc"
  );
  assert!(r.target.join(".vimrc").exists());
}

#[test]
fn create_path_flag_controls_layout() {
  let r = rig();
  write_file(&r.target, ".zshrc", "export EDITOR=vim");
  r.cmd()
    .args(["create", "zsh", "--yes", "--path", ".zshrc"])
    .assert()
    .success();
  assert_eq!(
    fs::read_to_string(r.source.join("zsh/.zshrc")).unwrap(),
    "export EDITOR=vim"
  );
  assert!(r.target.join(".zshrc").exists());
}

#[test]
fn create_existing_package_links_it_without_moving() {
  let r = rig();
  // package exists first, target path is free; --yes => link, never move
  write_file(&r.source, "vim/.config/vim/vimrc", "packaged");
  // pre-create .config so the package's dir folds at .config/vim level
  fs::create_dir_all(r.target.join(".config")).unwrap();
  r.cmd().args(["create", "vim", "--yes"]).assert().success();
  // package was not overwritten
  assert_eq!(
    fs::read_to_string(r.source.join("vim/.config/vim/vimrc")).unwrap(),
    "packaged"
  );
  // and the target path now resolves into the package (folded dir or file link)
  assert!(
    r.target.join(".config/vim").is_symlink() || r.target.join(".config/vim/vimrc").is_symlink()
  );
  assert_eq!(
    fs::read_to_string(r.target.join(".config/vim/vimrc")).unwrap(),
    "packaged"
  );
}

#[test]
fn create_existing_package_with_conflicting_target_aborts() {
  let r = rig();
  // package exists AND the target path is occupied by a real file —
  // a conflict; --yes does not imply overwriting (that is -f/--adopt)
  write_file(&r.source, "vim/.config/vim/vimrc", "packaged");
  write_file(&r.target, ".config/vim/vimrc", "target copy");
  r.cmd().args(["create", "vim", "--yes"]).assert().failure();
  assert_eq!(
    fs::read_to_string(r.target.join(".config/vim/vimrc")).unwrap(),
    "target copy"
  );
}

#[test]
fn create_existing_package_non_interactive_reports_and_does_not_move() {
  let r = rig();
  write_file(&r.source, "vim/.config/vim/vimrc", "packaged");
  write_file(&r.target, ".config/vim/vimrc", "target copy");
  // no --yes, no tty: error path, nothing moved
  r.cmd().args(["create", "vim"]).assert().failure();
  assert_eq!(
    fs::read_to_string(r.target.join(".config/vim/vimrc")).unwrap(),
    "target copy"
  );
}

#[test]
fn create_rejects_traversal_path() {
  let r = rig();
  r.cmd()
    .args(["create", "vim", "--empty", "--path", "../escape"])
    .assert()
    .failure();
  assert!(!r.source.join("escape").exists());
}

#[test]
fn create_rejects_absolute_path() {
  let r = rig();
  r.cmd()
    .args(["create", "vim", "--empty", "--path", "/etc/passwd-copy"])
    .assert()
    .failure();
  assert!(!r.source.join("vim").exists());
}

#[test]
fn create_verbose_shows_files() {
  let r = rig();
  write_file(&r.target, ".config/vim/a.vim", "a");
  write_file(&r.target, ".config/vim/b.vim", "bb");
  let out = r
    .cmd()
    .args(["create", "vim", "--yes", "-v"])
    .assert()
    .success();
  let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
  assert!(stderr.contains("a.vim"));
  assert!(stderr.contains("b.vim"));
}
