//! Task 17: config schema for `[package.<name>.root]` (ADR 0001, amended
//! 2026-11-20).
//!
//! Config load validates SHAPE only: keys stay inside the package, values are
//! absolute after `~` expansion, and trailing slashes are cosmetic. The mapping
//! kind (file vs directory) follows the source path's actual filesystem type at
//! plan time — Task 18 — so a missing source path is NOT a config-load error.
//!
//! Every fixture isolates HOME/XDG_*/DOTFILES_*; the real machine is untouched.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use rustow::config::{Config, normalize_root_mapping};
use rustow::elevate::{Elevation, authorize_with};
use rustow::error::Error;
use rustow::ignore::Ignore;
use rustow::link::{
  Action, ConflictReason, LinkInput, RootMappings, apply_plans, plan_link_all_with_roots,
};

fn parse(body: &str) -> Config {
  toml::from_str(body).expect("config must parse")
}

fn root_of(cfg: &Config, pkg: &str) -> BTreeMap<String, String> {
  cfg
    .package
    .get(pkg)
    .expect("package table must be present")
    .root
    .clone()
    .expect("root mapping must be present")
}

/// (Step 1) A file-style key (`configs/root`) and a directory-style key
/// (`configs`) both parse. Which kind each one is, is decided later from the
/// source path's filesystem type — not here.
#[test]
fn parses_file_and_dir_mappings() {
  let cfg = parse(
    "[package.snapper.root]\n\
     \"configs/root\" = \"/etc/snapper/root\"\n\
     \"configs\" = \"/etc/snapper\"\n",
  );
  let root = root_of(&cfg, "snapper");
  assert_eq!(
    root.get("configs/root").map(String::as_str),
    Some("/etc/snapper/root")
  );
  assert_eq!(
    root.get("configs").map(String::as_str),
    Some("/etc/snapper")
  );
  assert_eq!(root.len(), 2);
}

/// (Step 1) A `..` escape in a key is rejected with an error naming the key —
/// the same rule `create --path` uses to keep paths inside the package.
#[test]
fn root_key_dotdot_escape_is_config_error() {
  let err = toml::from_str::<Config>("[package.snapper.root]\n\"../etc\" = \"/etc\"\n")
    .expect_err("an escaping key must be rejected at load");
  let msg = err.to_string();
  assert!(
    msg.contains(".."),
    "error must name the offending key, got: {msg}"
  );
  assert!(
    msg.contains("escape"),
    "error must explain the escape, got: {msg}"
  );
}

/// (Step 1) An absolute key escapes the package too (same as `create --path`).
#[test]
fn root_key_absolute_is_config_error() {
  let err = toml::from_str::<Config>("[package.snapper.root]\n\"/etc\" = \"/etc\"\n")
    .expect_err("an absolute key must be rejected at load");
  assert!(
    err.to_string().contains("escape"),
    "error must explain the escape, got: {err}"
  );
}

/// (Step 1) A `.`-only key has no real component and is rejected.
#[test]
fn root_key_without_component_is_config_error() {
  let err = toml::from_str::<Config>("[package.snapper.root]\n\".\" = \"/etc\"\n")
    .expect_err("a '.' key must be rejected at load");
  assert!(
    err.to_string().contains("component"),
    "error must explain the missing component, got: {err}"
  );
}

/// (Step 1) A non-absolute value is rejected and the error names the mapping
/// key so a human can find the bad line.
#[test]
fn non_absolute_value_errors_naming_the_key() {
  let err = toml::from_str::<Config>("[package.snapper.root]\n\"configs\" = \"etc/snapper\"\n")
    .expect_err("a relative destination must be rejected");
  let msg = err.to_string();
  assert!(
    msg.contains("configs"),
    "error must name the mapping key, got: {msg}"
  );
  assert!(
    msg.contains("absolute"),
    "error must explain absoluteness, got: {msg}"
  );
}

/// (Step 1) A leading `~` expands to the supplied HOME and the result is an
/// absolute, normalized string. The helper is pure (HOME is passed in), so
/// this never reads the real machine's HOME.
#[test]
fn tilde_value_expands_to_home() {
  let home = Path::new("/tmp/opencode/rustow-task17-fake-home");
  let (key, value) = normalize_root_mapping("configs/", "~/snapper/", home).unwrap();
  assert_eq!(key, "configs");
  assert_eq!(value, "/tmp/opencode/rustow-task17-fake-home/snapper");

  // A bare `~` is the home directory itself.
  let (_, value) = normalize_root_mapping("cfg", "~", home).unwrap();
  assert_eq!(value, "/tmp/opencode/rustow-task17-fake-home");

  // `~user` is not a supported expansion: it stays relative and must fail.
  let err = normalize_root_mapping("cfg", "~someone/cfg", home).unwrap_err();
  assert!(err.to_string().contains("absolute"), "got: {err}");
}

/// (Step 1) Trailing slashes on a key or a value are cosmetic; all four spellings
/// produce the identical normalized map.
#[test]
fn trailing_slashes_are_cosmetic() {
  let plain = root_of(
    &parse("[package.snapper.root]\n\"configs\" = \"/etc/snapper\"\n"),
    "snapper",
  );
  let both = root_of(
    &parse("[package.snapper.root]\n\"configs/\" = \"/etc/snapper/\"\n"),
    "snapper",
  );
  let key_only = root_of(
    &parse("[package.snapper.root]\n\"configs/\" = \"/etc/snapper\"\n"),
    "snapper",
  );
  let value_only = root_of(
    &parse("[package.snapper.root]\n\"configs\" = \"/etc/snapper/\"\n"),
    "snapper",
  );
  assert_eq!(plain, both);
  assert_eq!(plain, key_only);
  assert_eq!(plain, value_only);
  assert_eq!(
    plain.get("configs").map(String::as_str),
    Some("/etc/snapper")
  );
}

/// (Step 1, amendment) Config load cannot know the source path's filesystem
/// type, so a mapped source that does not exist on disk still parses here;
/// rejecting it is plan-time work (Task 18).
#[test]
fn missing_source_path_is_not_a_config_error() {
  let cfg = parse("[package.snapper.root]\n\"configs/definitely-absent\" = \"/etc/snapper/x\"\n");
  assert!(
    root_of(&cfg, "snapper").contains_key("configs/definitely-absent"),
    "shape-only config load must accept a not-yet-resolved source path"
  );
}

/// `root` must be a table; a scalar is a config error that names the key.
#[test]
fn root_must_be_a_table() {
  let err = toml::from_str::<Config>("[package.snapper]\nroot = \"/etc\"\n")
    .expect_err("root must be a table");
  assert!(
    err.to_string().contains("root"),
    "error must point at the root key, got: {err}"
  );
}

/// Adding the `root` field must not weaken `deny_unknown_fields`.
#[test]
fn unknown_key_in_package_table_is_still_an_error() {
  let err = toml::from_str::<Config>("[package.snapper]\nbogus = 1\n")
    .expect_err("deny_unknown_fields must be preserved inside [package.x]");
  assert!(
    err.to_string().contains("bogus"),
    "error must name the unknown key, got: {err}"
  );
}

// --- CLI surface with fully isolated environment -------------------------

struct Rig {
  _tmp: tempfile::TempDir,
  source: PathBuf,
  xdg: PathBuf,
  home: PathBuf,
}

fn rig(config_body: &str) -> Rig {
  let base = Path::new("/tmp/opencode");
  let mut builder = tempfile::Builder::new();
  builder.prefix("rustow-task17-");
  let tmp = if base.is_dir() {
    builder.tempdir_in(base).unwrap()
  } else {
    builder.tempdir().unwrap()
  };

  let source = tmp.path().join("Configs");
  let xdg = tmp.path().join("xdg");
  let home = tmp.path().join("home");
  fs::create_dir_all(&source).unwrap();
  fs::create_dir_all(xdg.join("rustow")).unwrap();
  fs::create_dir_all(&home).unwrap();
  fs::write(xdg.join("rustow/config.toml"), config_body).unwrap();

  Rig {
    _tmp: tmp,
    source,
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
      .env("DOTFILES_TARGET", self.home.join("target"))
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
}

/// The `..` config error surfaces through the real binary.
#[test]
fn dotdot_key_surfaces_at_cli() {
  let r = rig("[package.snapper.root]\n\"../etc\" = \"/etc\"\n");
  let out = r.cmd().args(["status", "snapper"]).assert().failure();
  let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
  assert!(
    stderr.contains("..") && stderr.contains("escape"),
    "CLI error must name the escaping key, got: {stderr}"
  );
}

/// A `~` destination expands to an absolute path under the isolated HOME, so
/// config load succeeds. (Without `~` the same string would be relative and
/// rejected — pinned by `non_absolute_value_errors_naming_the_key`.)
#[test]
fn tilde_value_parses_under_isolated_home() {
  let r = rig("[package.snapper.root]\n\"configs\" = \"~/snapper\"\n");
  // Task 19 wires root mappings into the CLI, so the mapped source must exist
  // at plan time (a missing source is now a clean plan-time error).
  r.make_pkg("snapper", &["existing.conf", "configs/theme.conf"]);
  r.cmd()
    .args(["link", "--dry-run", "snapper"])
    .assert()
    .success();
}

// --- Task 18: planner produces root-mapped actions (ADR 0001, amended) -----
//
// These drive the public planner seam directly. Every fixture is a tempdir
// under /tmp/opencode; destinations are absolute paths inside that tempdir,
// never a real /etc. The source path's filesystem type decides the mapping
// kind; a missing source is a clean plan-time error.

struct PlanFixture {
  _tmp: tempfile::TempDir,
  source: PathBuf,
  target: PathBuf,
}

fn plan_fixture() -> PlanFixture {
  let base = Path::new("/tmp/opencode");
  let mut builder = tempfile::Builder::new();
  builder.prefix("rustow-task18-");
  let tmp = if base.is_dir() {
    builder.tempdir_in(base).unwrap()
  } else {
    builder.tempdir().unwrap()
  };
  let source = tmp.path().join("Configs");
  let target = tmp.path().join("home");
  fs::create_dir_all(&source).unwrap();
  fs::create_dir_all(&target).unwrap();
  PlanFixture {
    _tmp: tmp,
    source,
    target,
  }
}

impl PlanFixture {
  fn write(&self, pkg: &str, rel: &str) -> PathBuf {
    let p = self.source.join(pkg).join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(&p, format!("contents of {rel}")).unwrap();
    p
  }

  fn plan(&self, pkgs: &[&str], roots: &RootMappings) -> Vec<rustow::link::Plan> {
    let ignore = Ignore::new(&[]).unwrap();
    let pkgs: Vec<String> = pkgs.iter().map(|s| s.to_string()).collect();
    plan_link_all_with_roots(
      &LinkInput {
        source: &self.source,
        target: &self.target,
        pkgs: &pkgs,
        fold: true,
        fold_ignore: &[],
        ignore: &ignore,
      },
      roots,
    )
    .unwrap()
  }
}

fn roots(pkg: &str, mappings: &[(&str, &str)]) -> RootMappings {
  let mut all: RootMappings = HashMap::new();
  let inner: BTreeMap<String, String> = mappings
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
  all.insert(pkg.to_string(), inner);
  all
}

/// (Step 1a) A file source produces exactly one `RootLink` action aimed at the
/// absolute destination, and no ordinary link/fold/mkdir into the target.
#[test]
fn file_source_maps_to_one_root_link_action() {
  let f = plan_fixture();
  let src = f.write("app", "configs/root");
  let dest = f.target.join("etc/snapper/root");
  let plans = f.plan(
    &["app"],
    &roots("app", &[("configs/root", dest.to_str().unwrap())]),
  );

  assert_eq!(plans.len(), 1);
  let plan = &plans[0];
  assert!(plan.conflicts.is_empty(), "conflicts: {:?}", plan.conflicts);

  let root_actions: Vec<_> = plan
    .actions
    .iter()
    .filter(|a| matches!(a, Action::RootLink { .. } | Action::RootFold { .. }))
    .collect();
  assert_eq!(root_actions.len(), 1, "actions: {:?}", plan.actions);
  match &root_actions[0] {
    Action::RootLink { src: got_src, dst } => {
      assert_eq!(*got_src, src);
      assert_eq!(*dst, dest);
    }
    other => panic!("expected RootLink, got {other:?}"),
  }
  let mapped_target = f.target.join("configs/root");
  assert!(
    plan.actions.iter().all(|a| match a {
      Action::Link { dst, .. }
      | Action::Repair { dst, .. }
      | Action::Fold { dst, .. }
      | Action::Mkdir { dst } => *dst != mapped_target,
      _ => true,
    }),
    "mapped path must not also be linked into the target: {:?}",
    plan.actions
  );
  assert!(
    plan
      .actions
      .iter()
      .all(|a| !matches!(a, Action::Fold { dst, .. } if *dst == f.target.join("configs"))),
    "the parent directory must not fold over the mapped file: {:?}",
    plan.actions
  );
}

/// (Step 1b) A directory source — key/value with or without trailing slashes —
/// produces exactly one fold-style `RootFold` at `<value>/<basename(source-dir)>`.
#[test]
fn directory_source_maps_to_single_root_fold_action() {
  for (key, value) in [
    ("configs", "etc/snapper"),
    ("configs/", "etc/snapper"),
    ("configs", "etc/snapper/"),
    ("configs/", "etc/snapper/"),
  ] {
    let f = plan_fixture();
    let src_dir = f
      .write("app", "configs/root")
      .parent()
      .unwrap()
      .to_path_buf();
    let value_abs = f.target.join(value);
    let expected_dst = f.target.join("etc/snapper").join("configs");

    let plans = f.plan(
      &["app"],
      &roots("app", &[(key, value_abs.to_str().unwrap())]),
    );
    assert_eq!(plans.len(), 1);
    let plan = &plans[0];
    assert!(
      plan.conflicts.is_empty(),
      "key={key} value={value} conflicts: {:?}",
      plan.conflicts
    );

    let real_actions: Vec<_> = plan
      .actions
      .iter()
      .filter(|a| {
        matches!(
          a,
          Action::RootLink { .. }
            | Action::RootFold { .. }
            | Action::Fold { .. }
            | Action::Mkdir { .. }
            | Action::Link { .. }
        )
      })
      .collect();
    assert_eq!(
      real_actions.len(),
      1,
      "key={key} value={value} actions: {:?}",
      plan.actions
    );
    match &real_actions[0] {
      Action::RootFold { src_dir: got, dst } => {
        assert_eq!(*got, src_dir);
        assert_eq!(*dst, expected_dst);
      }
      other => panic!("expected RootFold, got {other:?}"),
    }
  }
}

/// (Step 1c) Root-mapped paths are removed from the ordinary walk, so they
/// never take part in the package's normal fold decisions — while a sibling
/// ordinary directory still folds normally.
#[test]
fn root_mapped_paths_absent_from_normal_fold_decisions() {
  let f = plan_fixture();
  f.write("app", "configs/root"); // root-mapped dir, has contents
  f.write("app", "other/theme.yml"); // ordinary dir, has contents
  let dest = f.target.join("etc/snapper");

  let plans = f.plan(
    &["app"],
    &roots("app", &[("configs", dest.to_str().unwrap())]),
  );
  let plan = &plans[0];
  let mapped_root = f.target.join("configs");

  assert!(
    plan.actions.iter().all(|a| match a {
      Action::Fold { dst, .. }
      | Action::Mkdir { dst }
      | Action::Link { dst, .. }
      | Action::Repair { dst, .. } => !dst.starts_with(&mapped_root),
      _ => true,
    }),
    "no ordinary action under the mapped path: {:?}",
    plan.actions
  );
  assert!(
    plan
      .actions
      .iter()
      .any(|a| matches!(a, Action::Fold { dst, .. } if *dst == f.target.join("other"))),
    "sibling ordinary directory must still fold: {:?}",
    plan.actions
  );
  assert!(
    plan.actions.iter().any(
      |a| matches!(a, Action::RootFold { src_dir, .. } if *src_dir == f.source.join("app/configs"))
    ),
    "root fold must be planned: {:?}",
    plan.actions
  );
}

/// (Step 1d) A real file occupying the absolute destination is a conflict;
/// two-phase apply aborts before writing anything, including ordinary links.
#[test]
fn conflict_at_absolute_destination_aborts_before_apply() {
  let f = plan_fixture();
  f.write("app", "configs/root");
  f.write("app", "other/theme.yml"); // ordinary link that must NOT be applied
  let dest = f.target.join("etc/snapper/root");
  fs::create_dir_all(dest.parent().unwrap()).unwrap();
  fs::write(&dest, "system file in the way").unwrap();

  let plans = f.plan(
    &["app"],
    &roots("app", &[("configs/root", dest.to_str().unwrap())]),
  );
  let plan = &plans[0];
  assert_eq!(plan.conflicts.len(), 1, "conflicts: {:?}", plan.conflicts);
  assert_eq!(plan.conflicts[0].dst, dest);
  assert_eq!(plan.conflicts[0].reason, ConflictReason::Exists);

  let mut resolver = rustow::link::AbortResolver;
  let err = apply_plans(&plans, &f.source, &mut resolver, false).unwrap_err();
  assert!(matches!(err, Error::Conflicts(_)), "err: {err}");
  assert!(
    !f.target.join("other/theme.yml").exists(),
    "ordinary links must not be applied when a root conflict aborts"
  );
  assert_eq!(
    fs::read_to_string(&dest).unwrap(),
    "system file in the way",
    "existing file must be untouched"
  );
}

/// (Step 1e) A mapped source path that does not exist is a clean plan-time
/// error — no actions, no fs changes.
#[test]
fn missing_mapped_source_is_clean_plan_time_error() {
  let f = plan_fixture();
  f.write("app", "other/theme.yml"); // package exists so validation passes
  let dest = f.target.join("etc/snapper/absent");
  let ignore = Ignore::new(&[]).unwrap();
  let pkgs = vec!["app".to_string()];

  let err = plan_link_all_with_roots(
    &LinkInput {
      source: &f.source,
      target: &f.target,
      pkgs: &pkgs,
      fold: true,
      fold_ignore: &[],
      ignore: &ignore,
    },
    &roots("app", &[("configs/absent", dest.to_str().unwrap())]),
  )
  .unwrap_err();

  let msg = err.to_string();
  assert!(
    msg.contains("configs/absent") && msg.contains("root"),
    "error must name the missing root source, got: {msg}"
  );
  assert!(!dest.exists(), "nothing applied");
}

// --- Task 19: apply + elevation with plan-time preflight (ADR 0001) -------
//
// CLI-level fixtures. Every path (including the "root" destination) lives in a
// tempdir under /tmp/opencode; the stub elevation program just `exec`s its argv
// as the invoking user. Real $HOME, ~/.stowrc and /etc are never touched.

struct ElevRig {
  _tmp: tempfile::TempDir,
  source: PathBuf,
  xdg: PathBuf,
  home: PathBuf,
  dest_parent: PathBuf,
  log: PathBuf,
}

fn elev_rig(with_stub: bool) -> ElevRig {
  let base = Path::new("/tmp/opencode");
  let mut builder = tempfile::Builder::new();
  builder.prefix("rustow-task19-");
  let tmp = if base.is_dir() {
    builder.tempdir_in(base).unwrap()
  } else {
    builder.tempdir().unwrap()
  };
  let source = tmp.path().join("Configs");
  let xdg = tmp.path().join("xdg");
  let home = tmp.path().join("home");
  let bin = tmp.path().join("bin");
  let dest_parent = tmp.path().join("system/etc/app");
  let stub = bin.join("elevate-shim");
  let log = bin.join("calls.log");
  fs::create_dir_all(&source).unwrap();
  fs::create_dir_all(xdg.join("rustow")).unwrap();
  fs::create_dir_all(&home).unwrap();
  fs::create_dir_all(&bin).unwrap();
  fs::create_dir_all(&dest_parent).unwrap();
  if with_stub {
    fs::write(
      &stub,
      format!("#!/bin/sh\necho \"$@\" >> {}\nexec \"$@\"\n", log.display()),
    )
    .unwrap();
    fs::set_permissions(&stub, fs::Permissions::from_mode(0o755)).unwrap();
  }
  let dest = dest_parent.join("app.ini");
  let body = format!(
    "elevate_cmd = {:?}\n\n[package.app.root]\n\"app.ini\" = {:?}\n",
    stub.to_str().unwrap(),
    dest.to_str().unwrap()
  );
  fs::write(xdg.join("rustow/config.toml"), body).unwrap();
  ElevRig {
    _tmp: tmp,
    source,
    xdg,
    home,
    dest_parent,
    log,
  }
}

impl ElevRig {
  fn cmd(&self) -> Command {
    let mut cmd = Command::cargo_bin("rustow").unwrap();
    cmd
      .env("XDG_CONFIG_HOME", &self.xdg)
      .env("XDG_CACHE_HOME", &self.xdg)
      .env("HOME", &self.home)
      .env("DOTFILES_DIR", &self.source)
      .env("DOTFILES_TARGET", self.home.join("target"))
      .env("NO_COLOR", "1");
    cmd
  }

  fn make_pkg(&self) {
    let root = self.source.join("app");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("app.ini"), "system ini").unwrap();
    fs::write(root.join(".zshrc"), "home zshrc").unwrap();
  }

  fn dest(&self) -> PathBuf {
    self.dest_parent.join("app.ini")
  }

  fn ordinary(&self) -> PathBuf {
    self.home.join("target/.zshrc")
  }

  fn set_conflict_policy(&self, policy: &str) {
    let path = self.xdg.join("rustow/config.toml");
    let body = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("on_conflict = {policy:?}\n{body}")).unwrap();
  }
}

fn stderr_of(out: &assert_cmd::assert::Assert) -> String {
  String::from_utf8(out.get_output().stderr.clone()).unwrap()
}

/// (a) Non-TTY without --as-root: stderr names --as-root and NOTHING changes.
#[test]
fn non_tty_without_as_root_aborts_with_hint() {
  let r = elev_rig(true);
  r.make_pkg();
  let out = r
    .cmd()
    .args(["link", "app"])
    .write_stdin("")
    .assert()
    .failure();
  let stderr = stderr_of(&out);
  assert!(
    stderr.contains("--as-root"),
    "stderr must name --as-root, got: {stderr}"
  );
  assert!(!r.dest().exists(), "root link must not be created");
  assert!(
    !r.ordinary().exists(),
    "zero filesystem changes: ordinary link must not be created"
  );
}

/// (b) --dry-run --as-root: nothing is applied, no elevation, no prompt.
#[test]
fn dry_run_with_as_root_applies_nothing() {
  let r = elev_rig(true);
  r.make_pkg();
  let out = r
    .cmd()
    .args(["link", "--dry-run", "--as-root", "app"])
    .assert()
    .success();
  assert!(!r.dest().exists(), "dry run must not create the root link");
  assert!(
    !r.ordinary().exists(),
    "dry run must not create ordinary link"
  );
  assert!(!r.log.exists(), "dry run must never invoke elevation");
  let stdout = String::from_utf8(out.get_output().stdout.clone()).unwrap();
  assert!(stdout.contains("would link"), "got: {stdout}");
}

/// (c) A stub elevate_cmd creates the root link via `ln`/`mkdir -p`; unlink
/// removes exactly the mapped link, while ordinary links stay ordinary.
#[test]
fn elevate_stub_applies_and_unlinks_root_mapped_path() {
  let r = elev_rig(true);
  r.make_pkg();
  r.cmd()
    .args(["link", "--as-root", "app"])
    .assert()
    .success();

  assert!(r.dest().is_symlink());
  assert_eq!(
    fs::read_link(r.dest()).unwrap(),
    r.source.join("app/app.ini")
  );
  assert!(
    r.ordinary().is_symlink(),
    "ordinary link stays with the user"
  );
  let log = fs::read_to_string(&r.log).unwrap();
  assert!(
    log.contains("mkdir -p"),
    "stub must create parent dirs: {log}"
  );
  assert!(log.contains("ln -s"), "stub must create the link: {log}");

  r.cmd()
    .args(["unlink", "--as-root", "app"])
    .assert()
    .success();
  assert!(
    !r.dest().exists() && !r.dest().is_symlink(),
    "root link removed"
  );
  assert!(!r.ordinary().exists(), "ordinary link removed");
  let log = fs::read_to_string(&r.log).unwrap();
  assert!(
    log.contains("rm -f"),
    "unlink must go through the elevation helper: {log}"
  );
}

/// (d) A nonexistent elevate_cmd is a clear plan-time error; nothing applied.
#[test]
fn nonexistent_elevate_cmd_fails_before_mutation() {
  let r = elev_rig(false);
  r.make_pkg();
  let out = r
    .cmd()
    .args(["link", "--as-root", "app"])
    .write_stdin("")
    .assert()
    .failure();
  let stderr = stderr_of(&out);
  assert!(
    stderr.contains("elevate_cmd") && stderr.contains("not executable"),
    "error must name the bad elevate_cmd, got: {stderr}"
  );
  assert!(!r.dest().exists());
  assert!(!r.ordinary().exists(), "zero filesystem changes");
}

/// (e) An unwritable destination parent aborts before ANY mutation.
#[test]
fn unwritable_destination_parent_aborts_before_mutation() {
  let r = elev_rig(true);
  r.make_pkg();
  fs::set_permissions(&r.dest_parent, fs::Permissions::from_mode(0o555)).unwrap();
  let out = r
    .cmd()
    .args(["link", "--as-root", "app"])
    .write_stdin("")
    .assert()
    .failure();
  let stderr = stderr_of(&out);
  assert!(
    stderr.contains("not writable"),
    "error must name the writability failure, got: {stderr}"
  );
  assert!(!r.dest().exists());
  assert!(
    !r.ordinary().exists(),
    "no ordinary link may be applied when preflight fails"
  );
  fs::set_permissions(&r.dest_parent, fs::Permissions::from_mode(0o755)).unwrap();
}

/// (f) Answering 'n' at the real y/n prompt aborts the whole run. Driven on a
/// pseudo-terminal (Linux `script`), since ADR 0001 now refuses to prompt on a
/// non-TTY at all.
#[cfg(target_os = "linux")]
#[test]
fn declining_elevation_prompt_aborts_everything() {
  use std::io::Write;
  use std::process::{Command as StdCommand, Stdio};
  let r = elev_rig(true);
  r.make_pkg();
  let bin = assert_cmd::cargo::cargo_bin("rustow");
  let mut child = StdCommand::new("script")
    .args(["-qec", &format!("{} link app", bin.display()), "/dev/null"])
    .env("XDG_CONFIG_HOME", &r.xdg)
    .env("XDG_CACHE_HOME", &r.xdg)
    .env("HOME", &r.home)
    .env("DOTFILES_DIR", &r.source)
    .env("DOTFILES_TARGET", r.home.join("target"))
    .env("NO_COLOR", "1")
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
  assert!(
    text.contains("proceed with elevated steps? [y/n]"),
    "prompt must be shown, got: {text}"
  );
  assert!(text.contains("aborted by user"), "got: {text}");
  assert!(!r.dest().exists());
  assert!(!r.ordinary().exists(), "zero filesystem changes");
}

/// Non-Linux: the PTY harness is unavailable, so assert the same decline at
/// the authorization seam.
#[cfg(not(target_os = "linux"))]
#[test]
fn declining_elevation_prompt_aborts_everything() {
  let mut input = std::io::Cursor::new(b"n\n".to_vec());
  let err =
    authorize_with(&["link /etc/app.ini".to_string()], false, true, &mut input).unwrap_err();
  assert!(matches!(err, Error::Aborted));
}

/// Review note #3: a root conflict resolved by Overwrite must stay on the
/// elevated path (not be converted into an ordinary `Action::Link`).
#[test]
fn root_conflict_overwrite_stays_elevated() {
  let r = elev_rig(true);
  r.make_pkg();
  fs::write(r.dest(), "system file in the way").unwrap();
  r.cmd()
    .args(["link", "--as-root", "--force", "app"])
    .assert()
    .success();
  assert!(r.dest().is_symlink(), "overwrite produced our root link");
  assert_eq!(
    fs::read_link(r.dest()).unwrap(),
    r.source.join("app/app.ini")
  );
  let log = fs::read_to_string(&r.log).unwrap();
  assert!(
    log.contains("ln -s"),
    "root overwrite must use elevation: {log}"
  );
}

/// Review note #2: two packages claiming the SAME absolute root destination
/// is a cross-package conflict, detected at plan time.
#[test]
fn duplicate_absolute_root_destination_is_cross_package_conflict() {
  let f = plan_fixture();
  f.write("a", "shared.ini");
  f.write("b", "shared.ini");
  let dest = f.target.join("etc/duplicate.ini");
  let mut all = RootMappings::new();
  all.insert(
    "a".to_string(),
    roots_inner(&[("shared.ini", dest.to_str().unwrap())]),
  );
  all.insert(
    "b".to_string(),
    roots_inner(&[("shared.ini", dest.to_str().unwrap())]),
  );
  let ignore = Ignore::new(&[]).unwrap();
  let pkgs = vec!["a".to_string(), "b".to_string()];
  let plans = plan_link_all_with_roots(
    &LinkInput {
      source: &f.source,
      target: &f.target,
      pkgs: &pkgs,
      fold: true,
      fold_ignore: &[],
      ignore: &ignore,
    },
    &all,
  )
  .unwrap();
  let conflicts: Vec<_> = plans
    .iter()
    .flat_map(|p| p.conflicts.iter().filter(|c| c.dst == dest))
    .collect();
  assert!(
    conflicts
      .iter()
      .any(|c| c.reason == ConflictReason::CrossPackage),
    "expected a cross-package root conflict, got: {conflicts:?}"
  );
}

fn roots_inner(mappings: &[(&str, &str)]) -> BTreeMap<String, String> {
  mappings
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect()
}

// --- Review fixes ----------------------------------------------------------

/// F1: a root conflict resolved by Adopt must be removed via elevation BEFORE
/// apply, and must warn that the system file was replaced, not adopted.
#[test]
fn root_adopt_replaces_with_explicit_warning() {
  let r = elev_rig(true);
  r.make_pkg();
  r.set_conflict_policy("adopt");
  fs::write(r.dest(), "system file in the way").unwrap();
  let out = r
    .cmd()
    .args(["link", "--as-root", "app"])
    .assert()
    .success();

  assert!(r.dest().is_symlink(), "elevated link must be created");
  assert_eq!(
    fs::read_link(r.dest()).unwrap(),
    r.source.join("app/app.ini")
  );
  assert!(r.ordinary().is_symlink(), "ordinary link still applied");
  let stderr = stderr_of(&out);
  assert!(
    stderr.contains("REPLACED") && stderr.contains("not adopted"),
    "a root adopt must warn that the file was replaced, got: {stderr}"
  );
  let log = fs::read_to_string(&r.log).unwrap();
  assert!(
    log.contains("rm -f"),
    "existing system file must be removed via elevation: {log}"
  );
}

/// A reader that panics if anything tries to read it — proves the non-TTY and
/// --as-root paths never block on an idle stdin pipe.
struct PanicOnRead;

impl std::io::Read for PanicOnRead {
  fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
    panic!("must not read stdin on non-TTY")
  }
}

impl std::io::BufRead for PanicOnRead {
  fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
    panic!("must not read stdin on non-TTY")
  }
  fn consume(&mut self, _amt: usize) {}
}

/// F2: a non-TTY fails loudly with the --as-root hint without reading stdin.
#[test]
fn non_tty_authorization_never_reads_idle_stdin() {
  let descriptions = vec!["link /etc/app.ini".to_string()];
  let err = authorize_with(&descriptions, false, false, &mut PanicOnRead).unwrap_err();
  assert!(matches!(err, Error::Config(_)), "got: {err}");
  assert!(
    err.to_string().contains("--as-root"),
    "error must name --as-root, got: {err}"
  );
}

/// F2: `--as-root` short-circuits without reading stdin; at an interactive
/// prompt EOF gives the same hint and 'n' aborts.
#[test]
fn authorization_seam_short_circuit_eof_and_decline() {
  let descriptions = vec!["link /etc/app.ini".to_string()];
  authorize_with(&descriptions, true, true, &mut PanicOnRead).unwrap();

  let mut eof = std::io::Cursor::new(Vec::new());
  let err = authorize_with(&descriptions, false, true, &mut eof).unwrap_err();
  assert!(matches!(err, Error::Config(_)), "got: {err}");
  assert!(err.to_string().contains("--as-root"), "got: {err}");

  let mut decline = std::io::Cursor::new(b"n\n".to_vec());
  let err = authorize_with(&descriptions, false, true, &mut decline).unwrap_err();
  assert!(matches!(err, Error::Aborted), "got: {err}");
}

/// F4: elevation passes paths as raw OsStr, so a non-UTF-8 destination works.
#[cfg(unix)]
#[test]
fn elevation_preserves_non_utf8_paths() {
  use std::ffi::OsStr;
  use std::os::unix::ffi::OsStrExt;
  let r = elev_rig(true);
  let stub = r._tmp.path().join("bin/elevate-shim");
  let elevation = Elevation::probe(Some(stub.to_str().unwrap())).unwrap();
  let src = r.source.join("app/app.ini");
  let dst = r.dest_parent.join(OsStr::from_bytes(b"app-\xff.ini"));
  elevation.apply_link(&src, &dst).unwrap();
  assert!(
    fs::symlink_metadata(&dst).unwrap().file_type().is_symlink(),
    "non-UTF-8 destination must be linked"
  );
  assert_eq!(fs::read_link(&dst).unwrap(), src);
}
