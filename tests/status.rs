//! `rustow status` integration tests.

use std::fs;
use std::path::Path;

use assert_cmd::Command;

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
  // empty per-fixture HOME: never read the real ~/.stowrc
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
fn status_linked_exit_zero() {
  let r = rig();
  make_pkg(&r.source, "eza", &[".config/eza/theme.yml"]);
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  r.cmd().args(["sync"]).assert().success();
  let out = r.cmd().args(["status"]).assert().success();
  let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
  assert!(text.contains("Package"));
  assert!(text.contains("eza"));
  assert!(text.contains("zsh"));
  assert!(text.contains("linked"));
}

#[test]
fn status_partial_exit_one() {
  let r = rig();
  make_pkg(&r.source, "zsh", &[".zshrc", ".config/zsh/rc.zsh"]);
  r.cmd().args(["link", "zsh"]).assert().success();
  // unlink one file manually, leaving the other linked
  fs::remove_file(r.target.join(".zshrc")).unwrap();
  let out = r.cmd().args(["status", "zsh"]).assert().failure();
  let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
  assert!(text.contains("partial"), "expected partial, got: {text}");
  assert!(text.contains("1 not linked"));
}

#[test]
fn status_unlinked_exit_one() {
  let r = rig();
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  let out = r.cmd().args(["status"]).assert().failure();
  let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
  assert!(text.contains("unlinked"));
}

#[test]
fn status_conflict_exit_one() {
  let r = rig();
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  fs::write(r.target.join(".zshrc"), "user contents").unwrap();
  let out = r.cmd().args(["status", "zsh"]).assert().failure();
  let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
  assert!(text.contains("conflict"), "expected conflict, got: {text}");
  assert!(text.contains("1 conflicts"));
}

#[test]
fn status_file_where_directory_expected_is_conflict() {
  let r = rig();
  // the package expects a directory at .config; a real file occupies it
  make_pkg(&r.source, "pkg", &[".config/eza/theme.yml"]);
  fs::write(r.target.join(".config"), "user contents").unwrap();
  let out = r.cmd().args(["status", "-v", "pkg"]).assert().failure();
  let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
  assert!(text.contains("conflict"), "expected conflict, got: {text}");
  assert!(
    text.contains("1 conflicts"),
    "expected one conflict, got: {text}"
  );
  assert!(
    text.contains("file where directory expected"),
    "expected kind-mismatch reason, got: {text}"
  );
}

#[test]
fn status_directory_where_leaf_expected_is_conflict() {
  let r = rig();
  // regression guard: this arm already reported a conflict before the dir-arm fix
  make_pkg(&r.source, "pkg", &[".zshrc"]);
  fs::create_dir(r.target.join(".zshrc")).unwrap();
  let out = r.cmd().args(["status", "-v", "pkg"]).assert().failure();
  let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
  assert!(text.contains("conflict"), "expected conflict, got: {text}");
  assert!(
    text.contains("1 conflicts"),
    "expected one conflict, got: {text}"
  );
  assert!(
    text.contains("is a real directory"),
    "expected leaf kind-mismatch reason, got: {text}"
  );
}

#[test]
fn status_filter_by_package() {
  let r = rig();
  make_pkg(&r.source, "eza", &[".config/eza/theme.yml"]);
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  r.cmd().args(["sync"]).assert().success();
  let out = r.cmd().args(["status", "zsh"]).assert().success();
  let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
  assert!(text.contains("zsh"));
  assert!(!text.contains("eza"));
}

#[test]
fn status_short_prints_summary_only() {
  let r = rig();
  make_pkg(&r.source, "eza", &[".config/eza/theme.yml"]);
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  r.cmd().args(["sync"]).assert().success();
  let out = r.cmd().args(["status", "-s"]).assert().success();
  let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
  assert!(text.contains("2 package(s)"));
  assert!(text.contains("2 linked"));
  assert!(!text.contains("Package")); // no table borders
}

#[test]
fn status_json_is_machine_readable() {
  let r = rig();
  make_pkg(&r.source, "eza", &[".config/eza/theme.yml"]);
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  r.cmd().args(["sync"]).assert().success();
  let out = r.cmd().args(["status", "--json"]).assert().success();
  let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
  let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
  assert_eq!(parsed["summary"]["total"], 2);
  assert_eq!(parsed["summary"]["linked"], 2);
  let names: Vec<&str> = parsed["packages"]
    .as_array()
    .unwrap()
    .iter()
    .map(|p| p["name"].as_str().unwrap())
    .collect();
  assert!(names.contains(&"eza") && names.contains(&"zsh"));
}

#[test]
fn status_json_has_top_level_version() {
  let r = rig();
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  r.cmd().args(["sync"]).assert().success();
  let out = r.cmd().args(["status", "--json"]).assert().success();
  let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
  let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
  assert_eq!(parsed["version"].as_u64(), Some(1));
}

#[test]
fn status_empty_package_shows_note() {
  let r = rig();
  fs::create_dir_all(r.source.join("ghost")).unwrap();
  let out = r.cmd().args(["status"]).assert().failure();
  let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
  assert!(text.contains("empty package"));
}

#[test]
fn status_has_no_status_specific_verbose() {
  let r = rig();
  // global -v/--verbose is fine (it is a global flag), but there is no status --verbose mode;
  // per-file detail comes from --json instead
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  r.cmd().args(["sync"]).assert().success();
  let out = r.cmd().args(["status", "--json"]).assert().success();
  let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
  let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
  assert!(parsed["packages"][0]["issues"].is_array());
}

#[test]
fn folded_package_reports_fully_linked() {
  let r = rig();
  make_pkg(
    &r.source,
    "eza",
    &[".config/eza/a.yml", ".config/eza/b.yml"],
  );
  r.cmd().args(["sync"]).assert().success();
  let out = r.cmd().args(["status"]).assert().success();
  let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
  assert!(text.contains("linked"));
  assert!(!text.contains("conflict"));
}

#[test]
fn status_fold_pointing_at_wrong_subtree_is_conflict() {
  let r = rig();
  // pkg has two subtrees under .config; craft a fold link that resolves to the
  // wrong one (.config/b) rather than the whole .config directory.
  make_pkg(&r.source, "pkg", &[".config/a", ".config/b/nested"]);
  let wrong_fold = r.source.join("pkg/.config/b");
  std::os::unix::fs::symlink(&wrong_fold, r.target.join(".config")).unwrap();

  let out = r.cmd().args(["status", "-v", "pkg"]).assert().failure();
  let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
  assert!(text.contains("conflict"), "expected conflict, got: {text}");
  assert!(
    text.contains(&format!("fold points at {}", wrong_fold.display())),
    "reason should name the wrong fold target, got: {text}"
  );
}

#[test]
fn dangling_link_counts_as_not_linked_not_crash() {
  let r = rig();
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  fs::remove_file(r.target.join(".zshrc")).unwrap_err(); // sanity: doesn't exist
  std::os::unix::fs::symlink(r.source.join("zsh/.zshrc"), r.target.join(".zshrc")).unwrap();
  // delete the package file, leaving a dangling stow-owned link
  fs::remove_file(r.source.join("zsh/.zshrc")).unwrap();
  let out = r.cmd().args(["status", "zsh"]).assert().failure();
  let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
  assert!(
    text.contains("not linked")
      || text.contains("dangling")
      || text.contains("partial")
      || text.contains("unlinked"),
    "got: {text}"
  );
}

#[test]
fn status_state_filter_linked_exit_zero() {
  let r = rig();
  make_pkg(&r.source, "eza", &[".config/eza/theme.yml"]);
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  r.cmd().args(["link", "eza"]).assert().success();
  let out = r
    .cmd()
    .args(["status", "--state", "linked"])
    .assert()
    .success();
  let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
  assert!(text.contains("eza"));
  assert!(!text.contains("zsh"));
}

#[test]
fn status_state_filter_conflicts_only() {
  let r = rig();
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  make_pkg(&r.source, "eza", &[".config/eza/theme.yml"]);
  fs::write(r.target.join(".zshrc"), "user contents").unwrap();
  let out = r
    .cmd()
    .args(["status", "--state", "conflict"])
    .assert()
    .failure();
  let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
  assert!(text.contains("zsh"));
  assert!(!text.contains("eza"));
}

#[test]
fn status_state_filter_union_and_json() {
  let r = rig();
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  make_pkg(&r.source, "eza", &[".config/eza/theme.yml"]);
  r.cmd().args(["link", "eza"]).assert().success();
  let out = r
    .cmd()
    .args([
      "status", "--state", "unlinked", "--state", "partial", "--json",
    ])
    .assert()
    .failure();
  let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
  let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
  let names: Vec<&str> = parsed["packages"]
    .as_array()
    .unwrap()
    .iter()
    .map(|p| p["name"].as_str().unwrap())
    .collect();
  assert!(names.contains(&"zsh"));
  assert!(!names.contains(&"eza"));
  assert_eq!(parsed["summary"]["total"], 1);
}

#[test]
fn status_state_filter_rejects_garbage() {
  let r = rig();
  r.cmd()
    .args(["status", "--state", "bogus"])
    .assert()
    .failure();
}

#[test]
fn status_verbose_shows_issue_lines() {
  let r = rig();
  make_pkg(&r.source, "zsh", &[".zshrc", ".config/zsh/rc.zsh"]);
  r.cmd().args(["link", "zsh"]).assert().success();
  fs::remove_file(r.target.join(".zshrc")).unwrap();
  let out = r.cmd().args(["status", "-v", "zsh"]).assert().failure();
  let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
  assert!(text.contains("zsh: partial"), "header missing, got: {text}");
  assert!(text.contains("missing"));
  assert!(text.contains(".zshrc"));
}

#[test]
fn status_verbose_caps_and_vv_uncaps() {
  let r = rig();
  let root = r.source.join("big");
  fs::create_dir_all(&root).unwrap();
  for i in 1..=23 {
    fs::write(root.join(format!(".file{i}")), "x").unwrap();
  }
  r.cmd().args(["link", "big"]).assert().success();
  for i in 1..=23 {
    fs::remove_file(r.target.join(format!(".file{i}"))).unwrap();
  }
  let capped = r.cmd().args(["status", "-v", "big"]).assert().failure();
  let text = String::from_utf8(capped.get_output().stdout.clone()).unwrap();
  assert_eq!(text.matches("missing").count(), 20);
  assert!(text.contains("... and 3 more"));

  let full = r.cmd().args(["status", "-vv", "big"]).assert().failure();
  let text = String::from_utf8(full.get_output().stdout.clone()).unwrap();
  assert_eq!(text.matches("missing").count(), 23);
  assert!(!text.contains("... and"));
}

#[test]
fn status_short_and_verbose_conflict() {
  let r = rig();
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  r.cmd().args(["status", "-s", "-v"]).assert().failure();
}

#[test]
fn status_state_filter_with_no_match_prints_message_not_table() {
  let r = rig();
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  r.cmd().args(["link", "zsh"]).assert().success();
  // nothing is partial: the filter empties the list
  let out = r
    .cmd()
    .args(["status", "--state", "partial"])
    .assert()
    .success();
  let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
  assert!(text.contains("no packages match --state partial"));
  assert!(!text.contains("╭"));
}

#[test]
fn status_state_filter_with_no_match_json_stays_valid() {
  let r = rig();
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  r.cmd().args(["link", "zsh"]).assert().success();
  let out = r
    .cmd()
    .args(["status", "--state", "partial", "--json"])
    .assert()
    .success();
  let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
  let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
  assert_eq!(parsed["packages"].as_array().unwrap().len(), 0);
  assert!(!text.contains("no packages match"));
}

#[test]
fn status_empty_source_prints_message_not_table() {
  let r = rig();
  let out = r.cmd().args(["status"]).assert().success();
  let text = String::from_utf8(out.get_output().stdout.clone()).unwrap();
  assert!(text.contains("no packages found"));
  assert!(!text.contains("╭"));
}
