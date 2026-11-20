//! F6: the generated/default config must advertise this project's canonical
//! repository — and no other repository URL — at the real CLI seam.

use std::fs;
use std::path::PathBuf;

use assert_cmd::Command;

const CANONICAL_REPO: &str = "https://github.com/AhmedOsman101/Rustow";

struct Rig {
  _tmp: tempfile::TempDir,
  xdg: PathBuf,
  home: PathBuf,
}

fn rig() -> Rig {
  let tmp = tempfile::tempdir().unwrap();
  let xdg = tmp.path().join("xdg");
  let home = tmp.path().join("home");
  fs::create_dir_all(&xdg).unwrap();
  fs::create_dir_all(&home).unwrap();
  Rig {
    _tmp: tmp,
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
}

/// The canonical URL appears once, and there is no second repository URL.
fn assert_canonical_only(text: &str, where_: &str) {
  assert!(
    text.contains(CANONICAL_REPO),
    "{where_} is missing the canonical repository URL {CANONICAL_REPO:?}"
  );
  assert_eq!(
    text.matches("github.com").count(),
    1,
    "{where_} must contain exactly one repository URL, found extras:\n{text}"
  );
  assert!(
    !text.contains("othman-moussa"),
    "{where_} still advertises the stale repository namespace"
  );
}

#[test]
fn config_default_uses_canonical_repository_url() {
  let r = rig();
  let out = r.cmd().args(["config", "default"]).output().unwrap();
  assert!(out.status.success());
  let text = String::from_utf8(out.stdout).unwrap();
  assert_canonical_only(&text, "`config default` stdout");
}

#[test]
fn config_init_writes_canonical_repository_url() {
  let r = rig();
  r.cmd().args(["config", "init"]).assert().success();
  let path = r.xdg.join("rustow/config.toml");
  let text = fs::read_to_string(&path).unwrap();
  assert_canonical_only(&text, "`config init` output file");
}
