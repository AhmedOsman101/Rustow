//! Regressions for the `clean` target walk: it must never sweep the source
//! tree, and it must honour ignore globs. CLI-level, tempdir fixtures only.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
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
  let target = tmp.path().join("home");
  // the layout the spec prescribes: source nested below the target root
  let source = target.join("dotfiles/Configs");
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

/// A broken symlink that lives *inside a package* and points into the source
/// tree must survive `clean` — the source tree is not part of the sweep.
#[test]
fn clean_never_descends_into_source() {
  let r = rig();
  let pkg = r.source.join("zsh");
  fs::create_dir_all(&pkg).unwrap();
  let broken = pkg.join("broken-in-source");
  symlink(pkg.join("missing-in-source"), &broken).unwrap();

  r.cmd()
    .args(["clean"])
    .assert()
    .success()
    .stdout(contains("deleted 0 broken link(s)"))
    .stdout(contains("broken-in-source").not());

  assert!(broken.is_symlink());
}

/// Ignore globs apply to the package-relative path of a candidate link: an
/// ignored broken link in the target is left alone, a non-ignored one is swept.
#[test]
fn clean_honours_ignore_rules() {
  let r = rig();
  let pkg = r.source.join("zsh");
  fs::create_dir_all(&pkg).unwrap();
  fs::create_dir_all(r.xdg.join("rustow")).unwrap();
  fs::write(
    r.xdg.join("rustow/config.toml"),
    format!(
      "source = \"{}\"\ntarget = \"{}\"\nignore = [\"*.tmp\"]\n",
      r.source.display(),
      r.target.display()
    ),
  )
  .unwrap();

  let ignored = r.target.join("ignored-link");
  symlink(pkg.join("secret.tmp"), &ignored).unwrap();
  let owned = r.target.join("owned-link");
  symlink(pkg.join("gone"), &owned).unwrap();

  r.cmd().args(["clean"]).assert().success();

  assert!(ignored.is_symlink());
  assert!(!owned.exists() && !owned.is_symlink());
}
