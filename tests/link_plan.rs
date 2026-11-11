//! Planner tests: tempdir fixtures, fake $HOME target, real package tree.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

use rustow::ignore::Ignore;
use rustow::link::{Action, ConflictReason, LinkInput, plan_link_all};

struct Fixture {
  _tmp: tempfile::TempDir,
  source: PathBuf,
  target: PathBuf,
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

fn write_file(root: &Path, rel: &str, contents: &str) {
  let path = root.join(rel);
  fs::create_dir_all(path.parent().unwrap()).unwrap();
  fs::write(path, contents).unwrap();
}

fn make_pkg(source: &Path, pkg: &str, files: &[&str]) {
  let root = source.join(pkg);
  fs::create_dir_all(&root).unwrap();
  for f in files {
    write_file(&root, f, &format!("contents of {f}"));
  }
}

fn link_all(f: &Fixture, pkgs: &[&str]) -> Vec<rustow::link::Plan> {
  let ignore = Ignore::new(&[]).unwrap();
  let pkgs: Vec<String> = pkgs.iter().map(|s| s.to_string()).collect();
  plan_link_all(&LinkInput {
    source: &f.source,
    target: &f.target,
    pkgs: &pkgs,
    fold: true,
    fold_ignore: &[],
    ignore: &ignore,
  })
  .unwrap()
}

/// Test-only simplified executor: assumes no conflicts. Handles Mkdir/Fold/Link/Repair/Unfold.
fn exec(plan: &rustow::link::Plan) {
  for action in &plan.actions {
    match action {
      Action::Mkdir { dst } => fs::create_dir_all(dst).unwrap(),
      Action::Fold { src_dir, dst } => {
        remove_if_exists(dst);
        symlink(src_dir, dst).unwrap();
      }
      Action::Link { src, dst } | Action::Repair { src, dst } => {
        remove_if_exists(dst);
        fs::create_dir_all(dst.parent().unwrap()).unwrap();
        symlink(src, dst).unwrap();
      }
      Action::Unfold { dst } => {
        fs::remove_file(dst).unwrap(); // symlink currently there
      }
      Action::SkipIdempotent { .. } => {}
    }
  }
}

fn remove_if_exists(p: &Path) {
  if p.is_symlink() || p.is_file() {
    fs::remove_file(p).unwrap();
  } else if p.is_dir() {
    fs::remove_dir_all(p).unwrap();
  }
}

#[test]
fn fold_when_target_missing() {
  let f = fixture();
  make_pkg(&f.source, "eza", &[".config/eza/theme.yml"]);
  let plans = link_all(&f, &["eza"]);
  assert_eq!(plans.len(), 1);
  assert!(
    plans[0].conflicts.is_empty(),
    "conflicts: {:?}",
    plans[0].conflicts
  );
  // .config is claimed only by eza → the whole .config folds into the package
  assert!(
    plans[0]
      .actions
      .iter()
      .any(|a| matches!(a, Action::Fold { dst, .. } if dst == &f.target.join(".config")))
  );
}

#[test]
fn recurse_when_target_config_exists() {
  let f = fixture();
  make_pkg(&f.source, "eza", &[".config/eza/theme.yml"]);
  // pre-existing real .config blocks folding at the top; deeper dir can still fold
  fs::create_dir_all(f.target.join(".config")).unwrap();
  let plans = link_all(&f, &["eza"]);
  assert!(plans[0].conflicts.is_empty());
  assert!(
    plans[0]
      .actions
      .iter()
      .any(|a| matches!(a, Action::Fold { dst, .. } if dst == &f.target.join(".config/eza")))
  );
  assert!(
    !plans[0]
      .actions
      .iter()
      .any(|a| matches!(a, Action::Fold { dst, .. } if dst == &f.target.join(".config")))
  );
}

#[test]
fn conflict_real_file_aborts_plan() {
  let f = fixture();
  make_pkg(&f.source, "eza", &[".config/eza/theme.yml"]);
  fs::create_dir_all(f.target.join(".config/eza")).unwrap();
  fs::write(f.target.join(".config/eza/theme.yml"), "user contents").unwrap();
  let plans = link_all(&f, &["eza"]);
  assert_eq!(plans[0].conflicts.len(), 1);
  assert_eq!(plans[0].conflicts[0].reason, ConflictReason::Exists);
  // planner must not have touched the file
  assert_eq!(
    fs::read_to_string(f.target.join(".config/eza/theme.yml")).unwrap(),
    "user contents"
  );
}

#[test]
fn conflict_external_symlink() {
  let f = fixture();
  make_pkg(&f.source, "eza", &[".config/eza/theme.yml"]);
  fs::create_dir_all(f.target.join(".config/eza")).unwrap();
  let other = f.target.join("elsewhere");
  fs::create_dir_all(&other).unwrap();
  fs::create_dir_all(f.target.join(".config/eza")).unwrap();
  symlink(&other, f.target.join(".config/eza/theme.yml")).unwrap();
  let plans = link_all(&f, &["eza"]);
  assert_eq!(plans[0].conflicts.len(), 1);
  assert_eq!(
    plans[0].conflicts[0].reason,
    ConflictReason::ExternalSymlink
  );
}

#[test]
fn conflict_wrong_package() {
  let f = fixture();
  fs::create_dir_all(f.target.join(".config")).unwrap();
  make_pkg(&f.source, "eza", &[".config/eza/icons.yml"]);
  make_pkg(&f.source, "other", &[".config/eza/theme.yml"]);
  // link other first (it folds .config/eza), then linking eza must unfold it
  let plans_other = link_all(&f, &["other"]);
  exec(&plans_other[0]);
  let plans_eza = link_all(&f, &["eza"]);
  assert!(plans_eza[0].conflicts.is_empty());
  assert!(
    plans_eza[0]
      .actions
      .iter()
      .any(|a| matches!(a, Action::Unfold { dst } if dst == &f.target.join(".config/eza")))
  );
}

#[test]
fn wrong_package_same_file_conflicts_during_unfold() {
  let f = fixture();
  fs::create_dir_all(f.target.join(".config")).unwrap();
  // both packages ship the SAME file at the SAME path — even after unfolding,
  // only one can own it; the later plan must flag a CrossPackage conflict
  make_pkg(&f.source, "other", &[".config/eza/theme.yml"]);
  make_pkg(&f.source, "eza", &[".config/eza/theme.yml"]);
  let plans_other = link_all(&f, &["other"]);
  exec(&plans_other[0]);
  let plans_eza = link_all(&f, &["eza"]);
  assert_eq!(plans_eza[0].conflicts.len(), 1);
  assert_eq!(
    plans_eza[0].conflicts[0].reason,
    ConflictReason::CrossPackage
  );
  assert_eq!(
    plans_eza[0].conflicts[0].dst,
    f.target.join(".config/eza/theme.yml")
  );
}

#[test]
fn leaf_into_wrong_package_conflicts() {
  let f = fixture();
  fs::create_dir_all(f.target.join(".config")).unwrap();
  // pkg "a" folds shared dir; pkg "b" has a FILE that would replace the folded link
  write_file(&f.source.join("a"), ".config/shared/x.conf", "x");
  write_file(&f.source.join("b"), ".config/shared/file-in-b.conf", "b");
  let plans_a = link_all(&f, &["a"]);
  exec(&plans_a[0]);
  let plans_b = link_all(&f, &["b"]);
  // .config/shared is folded to pkg a — b's file below it must NOT be linked into a's tree;
  // plan must unfold instead (this is the correct stow behaviour)
  assert!(
    plans_b[0].conflicts.is_empty(),
    "unexpected conflicts: {:?}",
    plans_b[0].conflicts
  );
  assert!(
    plans_b[0]
      .actions
      .iter()
      .any(|a| matches!(a, Action::Unfold { dst } if dst == &f.target.join(".config/shared")))
  );
}

#[test]
fn unfold_restores_both_packages() {
  let f = fixture();
  fs::create_dir_all(f.target.join(".config")).unwrap();
  write_file(&f.source.join("a"), ".config/shared/x.conf", "x");
  write_file(&f.source.join("b"), ".config/shared/y.conf", "y");
  // apply a (folds .config/shared)
  let plans_a = link_all(&f, &["a"]);
  exec(&plans_a[0]);
  assert!(f.target.join(".config/shared").is_symlink());
  // then b triggers unfold
  let plans_b = link_all(&f, &["b"]);
  exec(&plans_b[0]);
  // shared is now a real dir containing x.conf and y.conf, both symlinks
  assert!(f.target.join(".config/shared").is_dir());
  assert!(!f.target.join(".config/shared").is_symlink());
  assert_eq!(
    fs::read_to_string(f.target.join(".config/shared/x.conf")).unwrap(),
    "x"
  );
  assert_eq!(
    fs::read_to_string(f.target.join(".config/shared/y.conf")).unwrap(),
    "y"
  );
}

#[test]
fn file_vs_existing_real_dir_conflicts() {
  let f = fixture();
  write_file(&f.source.join("p"), "file-at-root", "contents");
  fs::create_dir_all(f.target.join("file-at-root")).unwrap();
  let plans = link_all(&f, &["p"]);
  assert_eq!(plans[0].conflicts.len(), 1);
  assert_eq!(
    plans[0].conflicts[0].reason,
    ConflictReason::DirFileMismatch
  );
}

#[test]
fn dir_vs_existing_real_file_conflicts() {
  let f = fixture();
  write_file(&f.source.join("p"), "somedir/child.txt", "contents");
  fs::write(f.target.join("somedir"), "not a dir").unwrap();
  let plans = link_all(&f, &["p"]);
  assert_eq!(plans[0].conflicts.len(), 1);
  assert_eq!(
    plans[0].conflicts[0].reason,
    ConflictReason::DirFileMismatch
  );
}

#[test]
fn ignored_only_package_is_noop_not_crash() {
  let f = fixture();
  // a package containing only a .git dir — every entry ignored
  write_file(
    &f.source.join("githeavy"),
    ".git/HEAD",
    "ref: refs/heads/main",
  );
  write_file(&f.source.join("githeavy"), ".git/objects/aa/bb", "blob");
  let plans = link_all(&f, &["githeavy"]);
  assert!(plans[0].conflicts.is_empty());
  assert!(plans[0].actions.is_empty());
  // status unaffected, no panic
}

#[test]
fn dangling_link_into_package_is_repaired() {
  let f = fixture();
  write_file(&f.source.join("eza"), ".config/eza/theme.yml", "theme");
  fs::create_dir_all(f.target.join(".config/eza")).unwrap();
  // stale dangling symlink pointing INTO this package (e.g. after moving the file)
  symlink(
    f.source.join("eza/.config/eza/deleted.yml"),
    f.target.join(".config/eza/theme.yml"),
  )
  .unwrap();
  let plans = link_all(&f, &["eza"]);
  assert!(plans[0].conflicts.is_empty());
  assert!(plans[0].actions.iter().any(
    |a| matches!(a, Action::Repair { dst, .. } if dst == &f.target.join(".config/eza/theme.yml"))
  ));
}

#[test]
fn idempotent_after_apply() {
  let f = fixture();
  make_pkg(
    &f.source,
    "eza",
    &[".config/eza/theme.yml", ".config/eza/icons.yml"],
  );
  let plans = link_all(&f, &["eza"]);
  exec(&plans[0]);
  let plans2 = link_all(&f, &["eza"]);
  assert!(plans2[0].conflicts.is_empty());
  assert!(
    plans2[0]
      .actions
      .iter()
      .all(|a| matches!(a, Action::SkipIdempotent { .. }))
      || plans2[0].actions.is_empty()
  );
}

#[test]
fn cross_package_same_file_conflict_flagged_for_second() {
  let f = fixture();
  write_file(&f.source.join("aaa"), ".zshrc", "aaa version");
  write_file(&f.source.join("bbb"), ".zshrc", "bbb version");
  let plans = link_all(&f, &["aaa", "bbb"]);
  // aaa wins; bbb flagged conflicted
  let bbb = plans.iter().find(|p| {
    p.conflicts
      .iter()
      .any(|c| c.reason == ConflictReason::CrossPackage)
  });
  assert!(
    bbb.is_some(),
    "expected a CrossPackage conflict: {:?}",
    plans
  );
  let dst = f.target.join(".zshrc");
  assert_eq!(
    plans
      .iter()
      .flat_map(|p| &p.conflicts)
      .filter(|c| c.dst == dst)
      .count(),
    1
  );
}

#[test]
fn cross_package_same_fold_dir_demoted_not_duplicated() {
  let f = fixture();
  // both packages ship .config/shared/ but with different files
  write_file(&f.source.join("aaa"), ".config/shared/a.conf", "a");
  write_file(&f.source.join("bbb"), ".config/shared/b.conf", "b");
  let plans = link_all(&f, &["aaa", "bbb"]);
  assert!(plans.iter().all(|p| p.conflicts.is_empty()));
  // neither package may fold .config/shared (both claim it) — per-file links instead
  for p in &plans {
    assert!(
      !p.actions
        .iter()
        .any(|a| matches!(a, Action::Fold { dst, .. } if dst == &f.target.join(".config/shared")))
    );
  }
  let all: Vec<_> = plans.iter().flat_map(|p| &p.actions).collect();
  assert!(all.iter().any(
    |a| matches!(a, Action::Link { dst, .. } if dst == &f.target.join(".config/shared/a.conf"))
  ));
  assert!(all.iter().any(
    |a| matches!(a, Action::Link { dst, .. } if dst == &f.target.join(".config/shared/b.conf"))
  ));
}

#[test]
fn env_home_expansion_is_not_done_by_planner() {
  // planner takes explicit paths; env expansion is the config layer's job
  let f = fixture();
  assert!(
    plan_link_all(&LinkInput {
      source: &f.source,
      target: &f.target,
      pkgs: &["nonexistent".to_string()],
      fold: true,
      fold_ignore: &[],
      ignore: &Ignore::new(&[]).unwrap(),
    })
    .is_err()
  );
}

#[test]
fn missing_package_errors() {
  let f = fixture();
  let err = plan_link_all(&LinkInput {
    source: &f.source,
    target: &f.target,
    pkgs: &["ghost".to_string()],
    fold: true,
    fold_ignore: &[],
    ignore: &Ignore::new(&[]).unwrap(),
  })
  .unwrap_err();
  assert!(err.to_string().contains("ghost"));
}

#[test]
fn fold_blocked_when_subtree_contains_ignored_paths() {
  let f = fixture();
  // .git inside a foldable dir — the package dir must NOT be folded;
  // per-file links are created and .git is skipped entirely
  write_file(&f.source.join("eza"), ".config/eza/theme.yml", "theme");
  write_file(&f.source.join("eza"), ".config/eza/.git/HEAD", "ref");
  fs::create_dir_all(f.target.join(".config")).unwrap();
  let plans = link_all(&f, &["eza"]);
  assert!(plans[0].conflicts.is_empty());
  assert!(
    !plans[0]
      .actions
      .iter()
      .any(|a| matches!(a, Action::Fold { dst, .. } if dst == &f.target.join(".config/eza")))
  );
  assert!(plans[0].actions.iter().any(
    |a| matches!(a, Action::Link { dst, .. } if dst == &f.target.join(".config/eza/theme.yml"))
  ));
  assert!(
    !plans[0]
      .actions
      .iter()
      .any(|a| matches!(a, Action::Link { dst, .. } if dst.to_string_lossy().contains(".git")))
  );
}

#[test]
fn fold_ignore_prevents_folding_for_listed_package() {
  let f = fixture();
  make_pkg(
    &f.source,
    "aha",
    &[".config/aha/theme.yml", ".config/aha/x.yml"],
  );
  make_pkg(&f.source, "nah", &[".zshrc"]);
  let ignore = Ignore::new(&[]).unwrap();
  let pkgs = vec!["aha".to_string(), "nah".to_string()];
  let fold_ignore = vec!["aha".to_string()]; // aha listed → no fold
  let plans = plan_link_all(&LinkInput {
    source: &f.source,
    target: &f.target,
    pkgs: &pkgs,
    fold: true,
    fold_ignore: &fold_ignore,
    ignore: &ignore,
  })
  .unwrap();
  let aha = plans.iter().find(|p| {
    p.actions
      .iter()
      .any(|a| matches!(a, Action::Mkdir { dst } if dst.ends_with("aha")))
  });
  assert!(
    aha.is_some(),
    "aha must be demoted to Mkdir+children, plans: {:?}",
    plans
  );
  assert!(
    !plans.iter().any(|p| p
      .actions
      .iter()
      .any(|a| matches!(a, Action::Fold { dst, .. } if dst == &f.target.join(".config/aha")))),
    "aha must not fold"
  );
}

#[test]
fn fold_ignore_does_not_block_unrelated_packages() {
  let f = fixture();
  make_pkg(&f.source, "aha", &[".config/aha/theme.yml"]);
  let ignore = Ignore::new(&[]).unwrap();
  let pkgs = vec!["aha".to_string()];
  let fold_ignore = vec!["nope".to_string()]; // aha NOT listed
  let plans = plan_link_all(&LinkInput {
    source: &f.source,
    target: &f.target,
    pkgs: &pkgs,
    fold: true,
    fold_ignore: &fold_ignore,
    ignore: &ignore,
  })
  .unwrap();
  assert!(
    plans[0]
      .actions
      .iter()
      .any(|a| matches!(a, Action::Fold { dst, .. } if dst == &f.target.join(".config")))
  );
}
