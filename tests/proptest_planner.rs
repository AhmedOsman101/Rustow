//! Property-based invariant tests for the linker's plan/apply/unlink seams.
//!
//! Every case builds a throwaway tree under /tmp/opencode with its own fake
//! HOME/target. The code under test takes explicit paths and never reads the
//! process environment, but we still point HOME/XDG_*/DOTFILES_* at inert
//! paths (once, before any case runs) so nothing can ever reach the real
//! dotfiles.

use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::Once;

use proptest::prelude::*;
use rustow::error::Error;
use rustow::ignore::Ignore;
use rustow::link::*;
use walkdir::WalkDir;

// --------------------------------------------------------------- environment

fn isolated_env() {
  static INIT: Once = Once::new();
  INIT.call_once(|| {
    let fake = PathBuf::from("/tmp/opencode/rustow-proptest-inert");
    fs::create_dir_all(fake.join("config")).unwrap();
    // SAFETY: the planner/apply/unlink seams take explicit paths and never read
    // these variables; this runs exactly once before any proptest case starts,
    // so there is no concurrent reader to race with.
    unsafe {
      std::env::set_var("HOME", &fake);
      std::env::set_var("XDG_CONFIG_HOME", fake.join("config"));
      std::env::set_var("XDG_CACHE_HOME", fake.join("config/cache"));
      std::env::set_var("DOTFILES_DIR", fake.join("no-source"));
      std::env::set_var("DOTFILES_TARGET", fake.join("no-target"));
    }
  });
}

// ------------------------------------------------------------------- fixtures

struct Fixture {
  _tmp: tempfile::TempDir,
  source: PathBuf,
  target: PathBuf,
}

fn fixture() -> Fixture {
  isolated_env();
  fs::create_dir_all("/tmp/opencode").unwrap();
  let tmp = tempfile::Builder::new()
    .prefix("rustow-proptest-")
    .tempdir_in("/tmp/opencode")
    .unwrap();
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

fn plan(f: &Fixture, pkgs: &[&str]) -> Vec<Plan> {
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

fn apply(
  f: &Fixture,
  plans: &[Plan],
  resolver: &mut dyn ConflictResolver,
) -> rustow::error::Result<ApplyOutcome> {
  apply_plans(plans, &f.source, resolver, false)
}

fn unlink(f: &Fixture, pkgs: &[&str]) {
  let ignore = Ignore::new(&[]).unwrap();
  let pkgs: Vec<String> = pkgs.iter().map(|s| s.to_string()).collect();
  let plan = plan_unlink(&f.source, &f.target, &pkgs, &ignore, false).unwrap();
  apply_unlink(&plan, &f.target, &f.source, &ignore, false).unwrap();
}

/// Deterministic description of a directory tree: kind, relative path, symlink
/// target / file bytes. Used to prove that operations are no-ops.
fn snapshot(root: &Path) -> Vec<String> {
  let mut out = Vec::new();
  for item in WalkDir::new(root).follow_links(false).into_iter().flatten() {
    let rel = item.path().strip_prefix(root).unwrap();
    if rel.as_os_str().is_empty() {
      continue;
    }
    let ft = item.file_type();
    if ft.is_symlink() {
      let target = fs::read_link(item.path()).unwrap();
      out.push(format!("L {} -> {}", rel.display(), target.display()));
    } else if ft.is_dir() {
      out.push(format!("D {}", rel.display()));
    } else {
      out.push(format!(
        "F {} {:?}",
        rel.display(),
        fs::read(item.path()).unwrap()
      ));
    }
  }
  out.sort();
  out
}

// --------------------------------------------------------- random tree model

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GenKind {
  File,
  Dir,
  Symlink,
}

#[derive(Debug, Clone, Copy)]
enum SymTarget {
  DanglingRel,
  DanglingUp,
  IntoPackage,
}

#[derive(Debug, Clone)]
struct GenEntry {
  rel: PathBuf,
  kind: GenKind,
  content: u8,
  sym_target: SymTarget,
}

/// A package tree with nested dirs, files, an occasional `.git`, and symlinks
/// of several spellings (dangling relative, dangling parent, into the package).
fn tree_strategy() -> impl Strategy<Value = Vec<GenEntry>> {
  let prefix = prop::sample::select(vec![
    vec![],
    vec![".config".to_string()],
    vec!["shared".to_string()],
    vec!["a".to_string(), "b".to_string()],
    vec![".git".to_string()],
    vec![".config".to_string(), "sub".to_string()],
  ]);
  prop::collection::vec((0u8..3, prefix.clone(), 0u8..4, 0u8..3), 0..8).prop_map(move |rows| {
    rows
      .into_iter()
      .enumerate()
      .map(|(i, (kind_sel, prefix, content, sym_sel))| {
        let (kind, leaf) = match kind_sel {
          0 => (GenKind::File, format!("f-{i}")),
          1 => (GenKind::Dir, format!("d-{i}")),
          _ => (GenKind::Symlink, format!("s-{i}")),
        };
        let mut rel = PathBuf::new();
        for component in &prefix {
          rel.push(component);
        }
        rel.push(leaf);
        let sym_target = match sym_sel {
          0 => SymTarget::DanglingRel,
          1 => SymTarget::DanglingUp,
          _ => SymTarget::IntoPackage,
        };
        GenEntry {
          rel,
          kind,
          content,
          sym_target,
        }
      })
      .collect()
  })
}

/// Materialize a generated tree into `source/<pkg>`; return the relative paths
/// of visible (non-ignored) entries, so properties can assert non-vacuity.
fn materialize(f: &Fixture, pkg: &str, entries: &[GenEntry]) -> Vec<PathBuf> {
  let root = f.source.join(pkg);
  fs::create_dir_all(&root).unwrap();
  let mut visible = Vec::new();
  let mut seen = HashSet::new();
  for entry in entries {
    if !seen.insert(entry.rel.clone()) {
      continue;
    }
    let abs = root.join(&entry.rel);
    fs::create_dir_all(abs.parent().unwrap()).unwrap();
    match entry.kind {
      GenKind::File => fs::write(&abs, [b'a' + entry.content]).unwrap(),
      GenKind::Dir => fs::create_dir_all(&abs).unwrap(),
      GenKind::Symlink => {
        let target = match entry.sym_target {
          SymTarget::DanglingRel => PathBuf::from("nowhere"),
          SymTarget::DanglingUp => PathBuf::from("../nowhere"),
          SymTarget::IntoPackage => root.join("pinned"),
        };
        symlink(target, &abs).unwrap();
      }
    }
    if !entry.rel.components().any(|c| c.as_os_str() == ".git") {
      visible.push(entry.rel.clone());
    }
  }
  visible
}

// ------------------------------------------------------------------ properties

/// Shared runner config. Failures here are test-fixture bugs, not persisted
/// adversarial inputs; don't scatter regression files or warn about source
/// discovery in CI. 256 cases per property is fast enough for the full suite.
fn prop_config() -> ProptestConfig {
  ProptestConfig {
    cases: 256,
    failure_persistence: Some(Box::new(proptest::test_runner::FileFailurePersistence::Off)),
    ..ProptestConfig::default()
  }
}

// (a) Random package trees (dirs, files, `.git`, symlinks) never panic in plan
// or apply. A panic is a proptest failure; we also assert the plan does real
// work so a tree of only ignored entries cannot make this vacuous.
proptest! {
  #![proptest_config(prop_config())]
  #[test]
  fn random_trees_never_panic_in_plan_or_apply(entries in tree_strategy()) {
    let f = fixture();
    let visible = materialize(&f, "p", &entries);
    prop_assume!(!visible.is_empty());

    let plans = plan(&f, &["p"]);
    prop_assert!(!plans[0].actions.is_empty(), "visible entries must produce actions");

    // `apply` returning Err is acceptable (the property is about panics), but
    // on this fresh fixture it should always succeed; a panic would fail here.
    let _ = apply(&f, &plans, &mut FixedResolver(ConflictResolution::Abort));
  }
}

// (b) A dry-run leaves the target tree byte-for-byte intact while reporting
// planned work as a dry run.
proptest! {
  #![proptest_config(prop_config())]
  #[test]
  fn dry_run_never_mutates(entries in tree_strategy()) {
    let f = fixture();
    let visible = materialize(&f, "p", &entries);
    prop_assume!(!visible.is_empty());

    let plans = plan(&f, &["p"]);
    let before = snapshot(&f.target);
    let outcome = apply_plans(
      &plans,
      &f.source,
      &mut FixedResolver(ConflictResolution::Abort),
      true,
    )
    .unwrap();
    prop_assert!(!plans[0].actions.is_empty(), "dry run must have work to report");
    prop_assert!(outcome.dry_run, "outcome must be marked as a dry run");
    prop_assert_eq!(snapshot(&f.target), before, "dry run mutated the target");
  }
}

// (c) Applying the same plan twice is a no-op the second time.
proptest! {
  #![proptest_config(prop_config())]
  #[test]
  fn link_is_idempotent(entries in tree_strategy()) {
    let f = fixture();
    let visible = materialize(&f, "p", &entries);
    prop_assume!(!visible.is_empty());

    let plans = plan(&f, &["p"]);
    let before = snapshot(&f.target);
    apply(&f, &plans, &mut FixedResolver(ConflictResolution::Abort)).unwrap();
    let after_first = snapshot(&f.target);
    prop_assert_ne!(&after_first, &before, "first apply must link something");
    apply(&f, &plans, &mut FixedResolver(ConflictResolution::Abort)).unwrap();
    prop_assert_eq!(&snapshot(&f.target), &after_first, "second apply changed the tree");
  }
}

// (d) An unresolved conflict aborts the whole run: the target is untouched.
proptest! {
  #![proptest_config(prop_config())]
  #[test]
  fn unresolved_conflict_changes_nothing(entries in tree_strategy()) {
    let f = fixture();
    let visible = materialize(&f, "p", &entries);
    let files: Vec<_> = visible
      .iter()
      .filter(|rel| f.source.join("p").join(rel).is_file())
      .collect();
    prop_assume!(!files.is_empty(), "need a file entry to occupy");
    let victim = files[0];
    // Pre-occupy a destination with a foreign real file to force a conflict.
    let dst = f.target.join(victim);
    fs::create_dir_all(dst.parent().unwrap()).unwrap();
    fs::write(&dst, b"foreign").unwrap();

    let plans = plan(&f, &["p"]);
    prop_assert!(
      plans.iter().any(|p| !p.conflicts.is_empty()),
      "test setup must create a conflict"
    );
    let before = snapshot(&f.target);
    let err = apply(&f, &plans, &mut AbortResolver).unwrap_err();
    prop_assert!(matches!(err, Error::Conflicts(_)), "expected a conflict abort");
    prop_assert_eq!(snapshot(&f.target), before, "conflicting apply mutated the target");
  }
}

// (e) Two packages sharing a dir: link a, link b (unfold), unlink b (refold)
// restores a valid fold to a; unlink a then empties the target.
proptest! {
  #![proptest_config(prop_config())]
  #[test]
  fn two_package_fold_unfold_refold_round_trip(
    dir in prop::sample::select(vec!["shared", "cfg", "bundle"]),
    file_a in prop::sample::select(vec!["x.conf", "theme.yml", "alpha"]),
    file_b in prop::sample::select(vec!["y.conf", "icons.yml", "beta"]),
    content_a in 0u8..8,
    content_b in 0u8..8,
  ) {
    prop_assume!(file_a != file_b);
    let f = fixture();
    let a = format!("contents-a-{content_a}");
    let b = format!("contents-b-{content_b}");
    let a_path = f.source.join("a").join(dir).join(file_a);
    let b_path = f.source.join("b").join(dir).join(file_b);
    fs::create_dir_all(a_path.parent().unwrap()).unwrap();
    fs::create_dir_all(b_path.parent().unwrap()).unwrap();
    fs::write(&a_path, a.as_bytes()).unwrap();
    fs::write(&b_path, b.as_bytes()).unwrap();

    // link a -> whole dir folds to a's subtree
    let plans = plan(&f, &["a"]);
    prop_assert!(plans[0].conflicts.is_empty());
    apply(&f, &plans, &mut FixedResolver(ConflictResolution::Abort)).unwrap();
    let dir_dst = f.target.join(dir);
    prop_assert!(dir_dst.is_symlink(), "a's dir should be a fold");
    prop_assert_eq!(fs::read_link(&dir_dst).unwrap(), f.source.join("a").join(dir));

    // link b -> unfolds into a real shared dir with both files linked
    let plans = plan(&f, &["b"]);
    prop_assert!(plans[0].conflicts.is_empty());
    apply(&f, &plans, &mut FixedResolver(ConflictResolution::Abort)).unwrap();
    prop_assert!(dir_dst.is_dir() && !dir_dst.is_symlink(), "b should unfold the dir");
    prop_assert_eq!(fs::read_to_string(dir_dst.join(file_a)).unwrap(), a.clone());
    prop_assert_eq!(fs::read_to_string(dir_dst.join(file_b)).unwrap(), b);

    // unlink b -> link-only dir refolds to a's subtree
    unlink(&f, &["b"]);
    prop_assert!(dir_dst.is_symlink(), "dir should refold after unlinking b");
    prop_assert_eq!(fs::read_link(&dir_dst).unwrap(), f.source.join("a").join(dir));
    prop_assert_eq!(fs::read_to_string(dir_dst.join(file_a)).unwrap(), a);
    prop_assert!(!dir_dst.join(file_b).exists(), "b's file must be gone");

    // unlink a -> empty target (no pre-existing dirs in this fixture)
    unlink(&f, &["a"]);
    prop_assert!(snapshot(&f.target).is_empty(), "target should be empty again");
  }
}

// (f) On a fresh target, unlink . link is the identity: every link created by
// link is removed and no residue remains.
proptest! {
  #![proptest_config(prop_config())]
  #[test]
  fn unlink_after_link_restores_fresh_target(entries in tree_strategy()) {
    let f = fixture();
    let visible = materialize(&f, "p", &entries);
    prop_assume!(!visible.is_empty());

    let before = snapshot(&f.target);
    let plans = plan(&f, &["p"]);
    prop_assert!(plans[0].conflicts.is_empty());
    apply(&f, &plans, &mut FixedResolver(ConflictResolution::Abort)).unwrap();
    prop_assert_ne!(&snapshot(&f.target), &before, "link must create entries");

    unlink(&f, &["p"]);
    prop_assert_eq!(&snapshot(&f.target), &before, "unlink must restore the fresh target");
  }
}
