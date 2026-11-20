//! Exit-code contract (ADR 0003), exercised through the real binary.
//!
//! 0 = success (status: every inspected package healthy), 1 = general failure
//! (conflicts, runtime errors, declining an interactive prompt), 2 = usage
//! error (clap). Every fixture uses a tempdir with isolated HOME / XDG_* /
//! DOTFILES_* so no real dotfiles or ~/.stowrc are ever touched.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use assert_cmd::Command as AssertCommand;
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
  fn cmd(&self) -> AssertCommand {
    let mut cmd = AssertCommand::cargo_bin("rustow").unwrap();
    cmd
      .env("HOME", &self.home)
      .env("XDG_CONFIG_HOME", &self.xdg)
      .env("XDG_CACHE_HOME", &self.xdg)
      .env_remove("DOTFILES_DIR")
      .env_remove("DOTFILES_TARGET");
    cmd
  }

  fn cmd_env(&self) -> AssertCommand {
    let mut cmd = self.cmd();
    cmd
      .env("DOTFILES_DIR", &self.source)
      .env("DOTFILES_TARGET", &self.target);
    cmd
  }

  /// A real config file keeps the TTY provenance prompt out of pty-driven
  /// tests, so the only prompt reached is the one under test.
  fn write_config(&self) {
    let dir = self.xdg.join("rustow");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
      dir.join("config.toml"),
      format!(
        "source = \"{}\"\ntarget = \"{}\"\n",
        self.source.display(),
        self.target.display()
      ),
    )
    .unwrap();
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
fn help_exits_zero() {
  let r = rig();
  r.cmd()
    .arg("--help")
    .assert()
    .code(0)
    .stdout(contains("Usage"));
}

#[test]
fn unknown_flag_exits_two() {
  let r = rig();
  r.cmd()
    .arg("--welp")
    .assert()
    .code(2)
    .stderr(contains("unexpected argument"));
}

#[test]
fn non_tty_conflict_exits_one() {
  let r = rig();
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  fs::write(r.target.join(".zshrc"), "user contents").unwrap();
  r.cmd_env()
    .args(["link", "zsh"])
    .assert()
    .code(1)
    .stderr(contains("conflict"));
  assert_eq!(
    fs::read_to_string(r.target.join(".zshrc")).unwrap(),
    "user contents"
  );
}

#[test]
fn conflict_decline_on_stdin_exits_one() {
  // A prompt-worthy conflict whose stdin can only supply "q" (no TTY) still
  // aborts with a named reason rather than returning a usage-style 2.
  let r = rig();
  r.write_config();
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  fs::write(r.target.join(".zshrc"), "user contents").unwrap();
  r.cmd()
    .write_stdin("q")
    .args(["link", "zsh"])
    .assert()
    .code(1)
    .stderr(contains("conflict"));
}

/// Drive the real binary on a pseudo-terminal (Linux `script` from util-linux)
/// so dialoguer actually prompts. Replying "n" to the default-no confirmation
/// declines it, reaching `Error::Aborted` — the path ADR 0003 folds into 1.
#[cfg(target_os = "linux")]
#[test]
fn interactive_decline_exits_one() {
  let r = rig();
  r.write_config();
  // An adoption source exists at the target, but the package does not — so
  // `create` asks "move ... into the package?" before touching anything.
  let src = r.target.join(".config/demo");
  fs::create_dir_all(&src).unwrap();
  fs::write(src.join("settings.toml"), "x = 1\n").unwrap();

  let bin = assert_cmd::cargo::cargo_bin("rustow");
  let mut child = Command::new("script")
    .args([
      "-qec",
      &format!("{} create demo", bin.display()),
      "/dev/null",
    ])
    .env("HOME", &r.home)
    .env("XDG_CONFIG_HOME", &r.xdg)
    .env("XDG_CACHE_HOME", &r.xdg)
    .env("DOTFILES_DIR", &r.source)
    .env("DOTFILES_TARGET", &r.target)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .unwrap();
  child.stdin.take().unwrap().write_all(b"n\n").unwrap();
  let out = child.wait_with_output().unwrap();

  assert_eq!(out.status.code(), Some(1), "output: {out:?}");
  let text = format!(
    "{}{}",
    String::from_utf8_lossy(&out.stdout),
    String::from_utf8_lossy(&out.stderr)
  );
  assert!(text.contains("aborted by user"), "output: {text}");
  // Declining must not move the source or create the package.
  assert!(src.join("settings.toml").exists());
  assert!(!r.source.join("demo").exists());
}

#[test]
fn status_unhealthy_exits_one() {
  let r = rig();
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  r.cmd_env().args(["status"]).assert().code(1);
}

#[test]
fn status_healthy_exits_zero() {
  let r = rig();
  make_pkg(&r.source, "zsh", &[".zshrc"]);
  r.cmd_env().args(["link", "zsh"]).assert().code(0);
  r.cmd_env().args(["status"]).assert().code(0);
}
