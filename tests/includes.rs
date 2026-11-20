//! Task 16: per-package `includes` drive the effective ignore at link time
//! (ADR 0002). These tests use the public CLI seam with isolated fixtures —
//! no real HOME, XDG_* or DOTFILES_* is ever read or written.

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

/// Build a fixture with `config.toml` containing `config_body`. `source` and
/// `target` are supplied through isolated DOTFILES_* env vars, never the real
/// machine's configuration.
fn rig(config_body: &str) -> Rig {
  let base = Path::new("/tmp/opencode");
  let mut builder = tempfile::Builder::new();
  builder.prefix("rustow-task16-");
  let tmp = if base.is_dir() {
    builder.tempdir_in(base).unwrap()
  } else {
    builder.tempdir().unwrap()
  };

  let source = tmp.path().join("Configs");
  let target = tmp.path().join("home");
  let xdg = tmp.path().join("xdg");
  let home = tmp.path().join("userhome");
  fs::create_dir_all(&source).unwrap();
  fs::create_dir_all(&target).unwrap();
  fs::create_dir_all(xdg.join("rustow")).unwrap();
  fs::create_dir_all(&home).unwrap();
  fs::write(xdg.join("rustow/config.toml"), config_body).unwrap();

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

  fn make_pkg(&self, pkg: &str, files: &[&str]) {
    let root = self.source.join(pkg);
    for f in files {
      let p = root.join(f);
      fs::create_dir_all(p.parent().unwrap()).unwrap();
      fs::write(p, format!("contents of {f}")).unwrap();
    }
  }

  fn dst(&self, rel: &str) -> PathBuf {
    self.target.join(rel)
  }
}

/// (a) A `!`-prefixed includes entry hides a matching path from linking.
#[test]
fn includes_hides_matching_path() {
  let r = rig("ignore = []\n\n[package.eza]\nincludes = [\"**\", \"!*.local\"]\n");
  r.make_pkg("eza", &["theme.yml", "theme.local"]);

  r.cmd().args(["link", "eza"]).assert().success();

  assert!(r.dst("theme.yml").is_symlink(), "kept file must be linked");
  assert!(
    !r.dst("theme.local").exists() && !r.dst("theme.local").is_symlink(),
    "!*.local must hide theme.local from linking"
  );
}

/// (b) A positive `**` re-includes a path that the global `ignore` hid.
#[test]
fn includes_double_star_reincludes_globally_hidden_path() {
  let r = rig("ignore = [\"*.bak\"]\n\n[package.eza]\nincludes = [\"**\"]\n");
  r.make_pkg("eza", &["keep.bak", "normal.txt"]);

  r.cmd().args(["link", "eza"]).assert().success();

  assert!(
    r.dst("keep.bak").is_symlink(),
    "** must re-include a path hidden by global ignore"
  );
  assert!(r.dst("normal.txt").is_symlink());
}

/// (c) A `!` entry after `**` re-hides the path again.
#[test]
fn includes_negation_rehides_reincluded_path() {
  let r = rig("ignore = [\"*.bak\"]\n\n[package.eza]\nincludes = [\"**\", \"!keep.bak\"]\n");
  r.make_pkg("eza", &["keep.bak", "other.bak", "normal.txt"]);

  r.cmd().args(["link", "eza"]).assert().success();

  assert!(
    r.dst("other.bak").is_symlink(),
    "the sibling .bak must be re-included by ** (proves the list is active)"
  );
  assert!(
    !r.dst("keep.bak").exists() && !r.dst("keep.bak").is_symlink(),
    "!keep.bak must re-hide a path that ** re-included"
  );
  assert!(r.dst("normal.txt").is_symlink());
}

/// (d) Effective hiding demotes folding: a subtree containing a hidden path
/// is linked per-file instead of being exposed as a single fold symlink.
#[test]
fn effective_hiding_demotes_folding() {
  let r = rig("ignore = []\n\n[package.eza]\nincludes = [\"**\", \"!**/*.local\"]\n");
  r.make_pkg("eza", &[".config/eza/theme.yml", ".config/eza/cache.local"]);

  r.cmd().args(["link", "eza"]).assert().success();

  assert!(
    r.dst(".config").is_dir() && !r.dst(".config").is_symlink(),
    ".config must not be folded over a hidden path"
  );
  assert!(
    r.dst(".config/eza").is_dir() && !r.dst(".config/eza").is_symlink(),
    ".config/eza must be demoted to a real directory"
  );
  assert!(r.dst(".config/eza/theme.yml").is_symlink());
  assert!(
    !r.dst(".config/eza/cache.local").exists() && !r.dst(".config/eza/cache.local").is_symlink(),
    "the hidden file must never be linked"
  );
}

/// (e) A package with no `includes` is completely unaffected by another
/// package's list.
#[test]
fn package_without_includes_is_unaffected() {
  let r = rig("ignore = []\n\n[package.alpha]\nincludes = [\"**\", \"!*.local\"]\n");
  r.make_pkg("alpha", &["a.txt", "a.local"]);
  r.make_pkg("beta", &["b.txt", "b.local"]);

  r.cmd().args(["link", "alpha", "beta"]).assert().success();

  assert!(r.dst("a.txt").is_symlink());
  assert!(
    !r.dst("a.local").exists() && !r.dst("a.local").is_symlink(),
    "alpha's includes must hide its own .local file"
  );
  assert!(r.dst("b.txt").is_symlink());
  assert!(
    r.dst("b.local").is_symlink(),
    "beta has no includes, so its .local file must still be linked"
  );
}

/// (f) Review Focus #3: a negated catch-all (`!**/*`, the `!**` family) hides
/// everything. Linking is an empty no-op, status reports the package unlinked,
/// and nothing panics.
#[test]
fn includes_hiding_everything_is_empty_noop_and_status_unlinked() {
  let r = rig("ignore = []\n\n[package.eza]\nincludes = [\"!**/*\"]\n");
  r.make_pkg("eza", &[".zshrc", ".config/eza/theme.yml"]);

  let out = r.cmd().args(["link", "eza"]).assert().success();
  let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
  assert!(
    stdout.contains("linked 0 path(s)"),
    "expected an empty link plan, got: {stdout}"
  );
  assert!(!r.dst(".zshrc").exists() && !r.dst(".zshrc").is_symlink());
  assert!(!r.dst(".config/eza/theme.yml").exists() && !r.dst(".config/eza/theme.yml").is_symlink());

  let status = r.cmd().args(["status", "eza"]).assert().failure();
  let text = String::from_utf8(status.get_output().stdout.clone()).unwrap();
  assert!(
    text.contains("unlinked"),
    "status must honestly report the unlinked package, got: {text}"
  );
}

/// (g) Refolding on unlink is folding too: when the only remaining links in a
/// shared directory belong to a package whose `includes` hides a file in that
/// subtree, a refold would expose the hidden file through a symlink. ADR 0002
/// requires the refold to be refused.
#[test]
fn refold_on_unlink_respects_package_includes() {
  let r = rig("ignore = []\n\n[package.alpha]\nincludes = [\"**\", \"!**/*.local\"]\n");
  r.make_pkg("alpha", &[".config/settings", ".config/cache.local"]);
  r.make_pkg("beta", &[".config/other"]);

  // Both packages claim .config, so it is linked per-file rather than folded.
  r.cmd().args(["link", "alpha", "beta"]).assert().success();
  assert!(r.dst(".config").is_dir() && !r.dst(".config").is_symlink());
  assert!(r.dst(".config/settings").is_symlink());
  assert!(r.dst(".config/other").is_symlink());
  assert!(
    !r.dst(".config/cache.local").exists(),
    "alpha's hidden file must not be linked in the first place"
  );

  // Unlinking beta leaves only alpha's link in .config; a refold to
  // source/<alpha>/.config would expose cache.local, which includes hide.
  r.cmd().args(["unlink", "beta"]).assert().success();

  assert!(
    !r.dst(".config").is_symlink(),
    "refold to alpha's .config must be refused: it would expose cache.local"
  );
  assert!(r.dst(".config").is_dir());
  assert!(r.dst(".config/settings").is_symlink());
  assert!(!r.dst(".config/cache.local").exists());
}
