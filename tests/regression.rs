//! Regression tests from an adversarial review of the linker core.
//! Each test names the finding it covers.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;

use assert_cmd::Command;
use predicates::str::contains;

struct Rig {
  _tmp: tempfile::TempDir,
  source: std::path::PathBuf,
  target: std::path::PathBuf,
  xdg: std::path::PathBuf,
  home: std::path::PathBuf,
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

// --- F1: --path must never resolve to the target root or above it
#[test]
fn create_path_dot_is_rejected() {
  let r = rig();
  for p in [".", "./", "./.", "", ".//", ".config/../."] {
    r.cmd()
      .args(["create", "app", "--empty", "--path", p])
      .assert()
      .failure();
    assert!(!r.source.join("app").exists(), "--path {p:?} got created");
  }
}

#[test]
fn create_path_with_only_dots_is_rejected() {
  let r = rig();
  r.cmd()
    .args(["create", "app", "--empty", "--path", ".config/../."])
    .assert()
    .failure();
}

#[test]
fn create_source_may_not_be_the_package_or_its_ancestor() {
  let r = rig();
  r.cmd()
    .args([
      "create",
      "app",
      "--yes",
      "--source",
      r.source.to_str().unwrap(),
    ])
    .assert()
    .failure();
}

// --- F2: overwrite never deletes a real directory tree
#[test]
fn force_does_not_delete_real_directory() {
  let r = rig();
  // package wants FILE at .config/app; target has a real dir there
  write_file(&r.source, "app/.config/app", "file content");
  fs::create_dir_all(r.target.join(".config/app/inner")).unwrap();
  fs::write(r.target.join(".config/app/inner/keep.txt"), "precious").unwrap();
  r.cmd().args(["link", "app", "-f"]).assert().success();
  assert!(
    r.target.join(".config/app/inner/keep.txt").exists(),
    "directory tree was deleted"
  );
  assert_eq!(
    fs::read_to_string(r.target.join(".config/app/inner/keep.txt")).unwrap(),
    "precious"
  );
  // and it must NOT have become a symlink to the package file
  assert!(!r.target.join(".config/app").is_symlink());
}

// --- F3: -f resolves cross-package same-target claims deterministically, no half-applied run
#[test]
fn force_cross_package_same_target_links_deterministically() {
  let r = rig();
  write_file(&r.source, "aaa/.zshrc", "aaa version");
  write_file(&r.source, "bbb/.zshrc", "bbb version");
  let out = r
    .cmd()
    .args(["link", "aaa", "bbb", "-f"])
    .assert()
    .success();
  let _ = out;
  // the conflicted (bbb) package's overwrite resolution wins deterministically
  assert!(r.target.join(".zshrc").is_symlink());
  let target = fs::read_link(r.target.join(".zshrc")).unwrap();
  assert!(
    target.ends_with("bbb/.zshrc"),
    "expected bbb to win, got {target:?}"
  );
  assert_eq!(
    fs::read_to_string(r.target.join(".zshrc")).unwrap(),
    "bbb version"
  );
}

#[test]
fn cross_package_conflict_without_flags_aborts_cleanly() {
  let r = rig();
  write_file(&r.source, "aaa/.zshrc", "aaa version");
  write_file(&r.source, "bbb/.zshrc", "bbb version");
  r.cmd().args(["link", "aaa", "bbb"]).assert().failure();
  assert!(!r.target.join(".zshrc").exists());
}

// --- F4: refold never exposes ignored data (nested .git) through a symlink
#[test]
fn refold_refuses_fold_when_ignored_paths_exist() {
  let r = rig();
  let pkg_b = r.source.join("bbb");
  write_file(&pkg_b, ".config/shared/y.conf", "b");
  write_file(&pkg_b, ".config/shared/.git/HEAD", "ref"); // ignored
  write_file(&r.source.join("aaa"), ".config/shared/x.conf", "a");
  fs::create_dir_all(r.target.join(".config")).unwrap();
  r.cmd().args(["link", "aaa"]).assert().success();
  r.cmd().args(["link", "bbb"]).assert().success(); // shared folds? no — fold blocked, per-file
  r.cmd().args(["unlink", "aaa"]).assert().success();
  // after unlinking aaa, shared must NOT collapse into a fold exposing .git
  assert!(!r.target.join(".config/shared").is_symlink());
  assert_eq!(
    fs::read_to_string(r.target.join(".config/shared/y.conf")).unwrap(),
    "b"
  );
  assert!(
    !r.target.join(".config/shared/.git/HEAD").exists(),
    "ignored .git must not be linked"
  );
}

// --- F5: a stale same-package fold on disk is repaired rather than erroring mid-run
#[test]
fn stale_owned_fold_is_replaced() {
  let r = rig();
  write_file(&r.source, "eza/.config/eza/theme.yml", "theme");
  fs::create_dir_all(r.target.join(".config")).unwrap();
  // stale fold: target/.config/eza points into the package but at a now-nonexistent dir
  symlink(
    r.source.join("eza/.config/renamed-away"),
    r.target.join(".config/eza"),
  )
  .unwrap();
  r.cmd().args(["link", "eza"]).assert().success();
  assert!(r.target.join(".config/eza").is_symlink());
  assert_eq!(
    fs::read_to_string(r.target.join(".config/eza/theme.yml")).unwrap(),
    "theme"
  );
}

// --- F6: package symlinks are linked as leaves, never descended into
#[test]
fn package_symlink_to_dir_is_linked_as_leaf() {
  let r = rig();
  // package contains .config/widgets -> /tmp/some-real-dir
  let real = r.target.join("elsewhere-dir");
  fs::create_dir_all(&real).unwrap();
  fs::write(real.join("secret.txt"), "classified").unwrap();
  fs::create_dir_all(r.source.join("pkg/.config")).unwrap();
  symlink(&real, r.source.join("pkg/.config/widgets")).unwrap();
  r.cmd().args(["link", "pkg"]).assert().success();
  assert!(r.target.join(".config/widgets").is_symlink());
  // the foreign file must NOT be reachable through our link chain
  // (it is reachable through the symlink — that's what a symlink does;
  //  the point is rustow must not create $HOME/secret.txt etc.)
  assert_eq!(
    fs::read_link(r.target.join(".config/widgets")).unwrap(),
    real
  );
}

// --- F8: package arguments are validated
#[test]
fn unlink_dotdot_is_rejected() {
  let r = rig();
  let out = r.cmd().args(["unlink", ".."]).assert().failure();
  let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
  assert!(stderr.contains("invalid package"), "stderr: {stderr}");
}

#[test]
fn link_dotdot_is_rejected() {
  let r = rig();
  r.cmd().args(["link", "../scripts"]).assert().failure();
  r.cmd().args(["link", ""]).assert().failure();
  r.cmd().args(["link", "."]).assert().failure();
}

// --- F10: dry run with conflicts is side-effect free
#[test]
fn dry_run_with_conflict_changes_nothing_and_reports() {
  let r = rig();
  write_file(&r.source, "zsh/.zshrc", "package");
  fs::write(r.target.join(".zshrc"), "user").unwrap();
  r.cmd()
    .args(["--dry-run", "link", "zsh"])
    .assert()
    .success();
  assert_eq!(fs::read_to_string(r.target.join(".zshrc")).unwrap(), "user");
  assert!(!r.target.join(".gitconfig").exists());
}

// --- F11: a symlink at the package destination never redirects the move
#[test]
fn create_force_with_symlink_at_destination_removes_symlink_not_target() {
  let r = rig();
  let outside = r.target.join("outside");
  fs::create_dir_all(&outside).unwrap();
  fs::write(outside.join("precious.txt"), "untouchable").unwrap();
  // package exists with .config/mytool being a symlink to outside
  fs::create_dir_all(r.source.join("mytool/.config")).unwrap();
  symlink(&outside, r.source.join("mytool/.config/mytool")).unwrap();
  write_file(&r.target, ".config/mytool/settings.conf", "data");
  r.cmd()
    .args(["create", "mytool", "--yes", "-f"])
    .assert()
    .success();
  // outside must be untouched; the symlink was replaced by real data
  assert!(outside.join("precious.txt").exists());
  assert_eq!(
    fs::read_to_string(outside.join("precious.txt")).unwrap(),
    "untouchable"
  );
  assert_eq!(
    fs::read_to_string(r.source.join("mytool/.config/mytool/settings.conf")).unwrap(),
    "data"
  );
}

#[test]
fn create_force_with_dangling_symlink_at_destination_works() {
  let r = rig();
  fs::create_dir_all(r.source.join("mytool/.config")).unwrap();
  symlink(
    r.source.join("mytool/.config/nowhere"),
    r.source.join("mytool/.config/mytool"),
  )
  .unwrap();
  write_file(&r.target, ".config/mytool/settings.conf", "data");
  r.cmd()
    .args(["create", "mytool", "--yes", "-f"])
    .assert()
    .success();
  assert_eq!(
    fs::read_to_string(r.source.join("mytool/.config/mytool/settings.conf")).unwrap(),
    "data"
  );
}

// --- F12: stowrc parsing
#[test]
fn stowrc_space_separated_form_parses() {
  let r = rig();
  write_file(&r.source, "zsh/.zshrc", "content");
  fs::write(
    r.home.join(".stowrc"),
    format!(
      "--dir {}\n--target {}\n",
      r.source.display(),
      r.target.display()
    ),
  )
  .unwrap();
  r.cmd()
    .env("HOME", &r.home)
    .args(["link", "zsh"])
    .assert()
    .success()
    .stderr(contains("no config.toml"));
  assert!(r.target.join(".zshrc").is_symlink());
}

#[test]
fn stowrc_missing_value_does_not_swallow_next_flag() {
  let r = rig();
  let stowrc = r.home.join(".stowrc");
  fs::write(&stowrc, "--dir\n--target /x\n").unwrap();
  // --dir has no value; must NOT take --target as the directory
  let out = r
    .cmd()
    .env("HOME", &r.home)
    .env_remove("DOTFILES_DIR")
    .env_remove("DOTFILES_TARGET")
    .args(["sync"])
    .output()
    .unwrap();
  assert!(!out.status.success());
  let stderr = String::from_utf8(out.stderr).unwrap();
  assert!(stderr.contains("source not set"), "stderr: {stderr}");
}

// --- F16: .. in target is normalized, not followed
#[test]
fn dotdot_in_target_is_normalized() {
  let r = rig();
  write_file(&r.source, "zsh/.zshrc", "x");
  // target with a `..` segment that normalizes right back into itself
  let dotdot = format!("{}/.config/..", r.target.display());
  r.cmd()
    .env("DOTFILES_TARGET", &dotdot)
    .args(["link", "zsh"])
    .assert()
    .success();
  assert!(r.target.join(".zshrc").is_symlink());
}
