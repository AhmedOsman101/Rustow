//! Link/apply/unlink integration tests against tempdir fixtures.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;

use rustow::ignore::Ignore;
use rustow::link::*;

struct Fixture {
  _tmp: tempfile::TempDir,
  source: std::path::PathBuf,
  target: std::path::PathBuf,
}

fn fixture() -> Fixture {
  let tmp = tempfile::tempdir().unwrap();
  let source = tmp.path().join("Configs");
  let target = tmp.path().join("home");
  fs::create_dir_all(&source).unwrap();
  fs::create_dir_all(&target).unwrap();
  Fixture {
    _tmp: tmp,
    source,
    target,
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

fn link(f: &Fixture, pkgs: &[&str]) {
  let ignore = Ignore::new(&[]).unwrap();
  let pkgs: Vec<String> = pkgs.iter().map(|s| s.to_string()).collect();
  let plans = plan_link_all(&LinkInput {
    source: &f.source,
    target: &f.target,
    pkgs: &pkgs,
    fold: true,
    ignore: &ignore,
  })
  .unwrap();
  let conflicts: Vec<_> = plans.iter().flat_map(|p| &p.conflicts).collect();
  assert!(conflicts.is_empty(), "unexpected conflicts: {conflicts:?}");
  apply_plans(
    &plans,
    &f.source,
    &mut FixedResolver(ConflictResolution::Abort),
    false,
  )
  .unwrap();
}

fn unlink(f: &Fixture, pkgs: &[&str], dry_run: bool) -> usize {
  let ignore = Ignore::new(&[]).unwrap();
  let pkgs: Vec<String> = pkgs.iter().map(|s| s.to_string()).collect();
  let plan = plan_unlink(&f.source, &f.target, &pkgs, &ignore).unwrap();
  apply_unlink(&plan, &f.target, &f.source, &ignore, dry_run).unwrap()
}

#[test]
fn link_then_unlink_restores_home() {
  let f = fixture();
  make_pkg(&f.source, "eza", &[".config/eza/theme.yml", ".zshrc"]);
  link(&f, &["eza"]);
  assert_eq!(
    fs::read_to_string(f.target.join(".zshrc")).unwrap(),
    "contents of .zshrc"
  );
  assert_eq!(
    fs::read_to_string(f.target.join(".config/eza/theme.yml")).unwrap(),
    "contents of .config/eza/theme.yml"
  );
  unlink(&f, &["eza"], false);
  assert!(!f.target.join(".zshrc").exists() && !f.target.join(".zshrc").is_symlink());
  assert!(!f.target.join(".config").exists()); // we created it; fully rolled back
  assert!(f.source.join("eza/.config/eza/theme.yml").exists()); // package untouched
}

#[test]
fn unlink_keeps_preexisting_dirs_and_foreign_files() {
  let f = fixture();
  // .config pre-exists; a foreign file sits next to our link
  fs::create_dir_all(f.target.join(".config")).unwrap();
  fs::write(f.target.join(".config/keep.txt"), "mine").unwrap();
  make_pkg(&f.source, "eza", &[".config/eza/theme.yml"]);
  link(&f, &["eza"]);
  unlink(&f, &["eza"], false);
  assert!(f.target.join(".config/keep.txt").exists());
  assert!(f.target.join(".config").is_dir());
  assert_eq!(
    fs::read_to_string(f.target.join(".config/keep.txt")).unwrap(),
    "mine"
  );
}

#[test]
fn unlink_never_removes_foreign_symlink() {
  let f = fixture();
  make_pkg(&f.source, "eza", &[".config/eza/theme.yml"]);
  // foreign link with the same destination path
  fs::create_dir_all(f.target.join(".config/eza")).unwrap();
  let elsewhere = f.target.join("elsewhere.txt");
  fs::write(&elsewhere, "not yours").unwrap();
  symlink(&elsewhere, f.target.join(".config/eza/theme.yml")).unwrap();
  // link would conflict, so force it: overwrite the foreign link via Overwrite resolver
  let ignore = Ignore::new(&[]).unwrap();
  let pkgs = vec!["eza".to_string()];
  let plans = plan_link_all(&LinkInput {
    source: &f.source,
    target: &f.target,
    pkgs: &pkgs,
    fold: true,
    ignore: &ignore,
  })
  .unwrap();
  apply_plans(
    &plans,
    &f.source,
    &mut FixedResolver(ConflictResolution::Overwrite),
    false,
  )
  .unwrap();
  // now unlink — the file is OUR link; foreign is gone by overwrite semantics. restore scenario:
  // foreign restored:
  fs::remove_file(f.target.join(".config/eza/theme.yml")).unwrap();
  symlink(&elsewhere, f.target.join(".config/eza/theme.yml")).unwrap();
  unlink(&f, &["eza"], false);
  // foreign link must still be there
  assert!(f.target.join(".config/eza/theme.yml").is_symlink());
  assert_eq!(
    fs::read_link(f.target.join(".config/eza/theme.yml")).unwrap(),
    elsewhere
  );
}

#[test]
fn dry_run_changes_nothing() {
  let f = fixture();
  make_pkg(&f.source, "eza", &[".config/eza/theme.yml"]);
  let ignore = Ignore::new(&[]).unwrap();
  let pkgs = vec!["eza".to_string()];
  let plans = plan_link_all(&LinkInput {
    source: &f.source,
    target: &f.target,
    pkgs: &pkgs,
    fold: true,
    ignore: &ignore,
  })
  .unwrap();
  let out = apply_plans(
    &plans,
    &f.source,
    &mut FixedResolver(ConflictResolution::Abort),
    true,
  )
  .unwrap();
  assert!(out.linked > 0);
  assert!(!f.target.join(".config").exists());
}

#[test]
fn unfold_then_unlink_refolds() {
  let f = fixture();
  // two packages, same dir, disjoint files → unfold into shared real dir
  fs::create_dir_all(f.target.join(".config")).unwrap();
  make_pkg(&f.source, "a", &[".config/shared/x.conf"]);
  make_pkg(&f.source, "b", &[".config/shared/y.conf"]);
  link(&f, &["a"]);
  assert!(f.target.join(".config/shared").is_symlink());
  link(&f, &["b"]); // unfolds
  assert!(f.target.join(".config/shared").is_dir());
  assert!(!f.target.join(".config/shared").is_symlink());
  // unlink b → stow re-folds the link-only dirs: shared (→ a's subtree), then
  // .config itself (all its remaining children resolve into a's .config)
  unlink(&f, &["b"], false);
  assert!(f.target.join(".config").is_symlink());
  assert_eq!(
    fs::read_link(f.target.join(".config")).unwrap(),
    f.source.join("a/.config")
  );
  // a's content remains reachable through the fold
  assert_eq!(
    fs::read_to_string(f.target.join(".config/shared/x.conf")).unwrap(),
    "contents of .config/shared/x.conf"
  );
  // unlink a → everything gone except preexisting .config
  // stow -D refolds parents too: after unlink b, .config folded to a/.config;
  // unlinking a removes the fold — nothing pre-existed to keep
  unlink(&f, &["a"], false);
  assert!(!f.target.join(".config").exists());
}

#[test]
fn conflict_overwrite_resolution_replaces_real_file() {
  let f = fixture();
  fs::create_dir_all(f.target.join(".config/eza")).unwrap();
  fs::write(f.target.join(".config/eza/theme.yml"), "user contents").unwrap();
  make_pkg(&f.source, "eza", &[".config/eza/theme.yml"]);
  let ignore = Ignore::new(&[]).unwrap();
  let pkgs = vec!["eza".to_string()];
  let plans = plan_link_all(&LinkInput {
    source: &f.source,
    target: &f.target,
    pkgs: &pkgs,
    fold: true,
    ignore: &ignore,
  })
  .unwrap();
  apply_plans(
    &plans,
    &f.source,
    &mut FixedResolver(ConflictResolution::Overwrite),
    false,
  )
  .unwrap();
  assert!(f.target.join(".config/eza/theme.yml").is_symlink());
  assert_eq!(
    fs::read_to_string(f.target.join(".config/eza/theme.yml")).unwrap(),
    "contents of .config/eza/theme.yml"
  );
}

#[test]
fn conflict_adopt_moves_target_into_package() {
  let f = fixture();
  fs::create_dir_all(f.target.join(".config/eza")).unwrap();
  fs::write(f.target.join(".config/eza/theme.yml"), "user contents").unwrap();
  make_pkg(&f.source, "eza", &[".config/eza/theme.yml"]);
  let ignore = Ignore::new(&[]).unwrap();
  let pkgs = vec!["eza".to_string()];
  let plans = plan_link_all(&LinkInput {
    source: &f.source,
    target: &f.target,
    pkgs: &pkgs,
    fold: true,
    ignore: &ignore,
  })
  .unwrap();
  apply_plans(
    &plans,
    &f.source,
    &mut FixedResolver(ConflictResolution::Adopt),
    false,
  )
  .unwrap();
  // package file now holds the user's contents; target is a symlink
  assert_eq!(
    fs::read_to_string(f.source.join("eza/.config/eza/theme.yml")).unwrap(),
    "user contents"
  );
  assert!(f.target.join(".config/eza/theme.yml").is_symlink());
}

#[test]
fn conflict_skip_leaves_target_alone() {
  let f = fixture();
  fs::create_dir_all(f.target.join(".config/eza")).unwrap();
  fs::write(f.target.join(".config/eza/theme.yml"), "user contents").unwrap();
  make_pkg(
    &f.source,
    "eza",
    &[".config/eza/theme.yml", ".config/eza/icons.yml"],
  );
  let ignore = Ignore::new(&[]).unwrap();
  let pkgs = vec!["eza".to_string()];
  let plans = plan_link_all(&LinkInput {
    source: &f.source,
    target: &f.target,
    pkgs: &pkgs,
    fold: true,
    ignore: &ignore,
  })
  .unwrap();
  apply_plans(
    &plans,
    &f.source,
    &mut FixedResolver(ConflictResolution::Skip),
    false,
  )
  .unwrap();
  assert_eq!(
    fs::read_to_string(f.target.join(".config/eza/theme.yml")).unwrap(),
    "user contents"
  );
  assert!(f.target.join(".config/eza/icons.yml").is_symlink());
}

#[test]
fn abort_resolver_changes_nothing() {
  let f = fixture();
  fs::create_dir_all(f.target.join(".config/eza")).unwrap();
  fs::write(f.target.join(".config/eza/theme.yml"), "user contents").unwrap();
  make_pkg(&f.source, "eza", &[".config/eza/theme.yml"]);
  let ignore = Ignore::new(&[]).unwrap();
  let pkgs = vec!["eza".to_string()];
  let plans = plan_link_all(&LinkInput {
    source: &f.source,
    target: &f.target,
    pkgs: &pkgs,
    fold: true,
    ignore: &ignore,
  })
  .unwrap();
  let err = apply_plans(&plans, &f.source, &mut AbortResolver, false).unwrap_err();
  assert!(matches!(err, rustow::error::Error::Conflicts(_)));
  assert_eq!(
    fs::read_to_string(f.target.join(".config/eza/theme.yml")).unwrap(),
    "user contents"
  );
  assert!(!f.target.join(".config/eza/icons.yml").exists());
}
