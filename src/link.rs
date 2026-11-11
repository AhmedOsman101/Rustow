//! Linker core: two-phase plan/apply for link / unlink / relink.
//!
//! Phases per run:
//!   1. scan each package into entries (respecting ignores),
//!   2. cross-package pre-pass over all entries: fold destinations claimed by
//!      another package (at or below) are demoted to per-file links, and
//!      same-target collisions across packages are flagged as conflicts
//!      up front (so the run can abort before touching the fs),
//!   3. recursive planning per package, unfolding rustow/stow-owned folded
//!      symlinks encountered on disk.
//!
//! The planner writes nothing. Applying is idempotent and ordered
//! (Unfold before children, Mkdir before descendants).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::error::{Error, Result};
use crate::ignore::Ignore;
use crate::platform;

// ---------------------------------------------------------------- entries

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
  File,
  Dir,
  Symlink,
}

#[derive(Debug, Clone)]
pub struct Entry {
  /// Path relative to the package root (`.config/eza/theme.yml`).
  pub pkg_rel: PathBuf,
  /// Absolute path of the source (in the package).
  pub abs_src: PathBuf,
  pub kind: EntryKind,
}

pub fn scan_package(source: &Path, pkg: &str, ignore: &Ignore) -> Result<Vec<Entry>> {
  let root = source.join(pkg);
  if !root.is_dir() {
    return Err(Error::PackageNotFound(pkg.to_string()));
  }

  let mut out = Vec::new();
  for item in WalkDir::new(&root).follow_links(false) {
    let entry = item.map_err(|e| Error::Message(e.to_string()))?;
    let rel = match entry.path().strip_prefix(&root) {
      Ok(p) if p.as_os_str().is_empty() => continue, // package root
      Ok(p) => p.to_path_buf(),
      Err(_) => continue,
    };
    if ignore.is_ignored(&rel) {
      continue;
    }
    let ft = entry.file_type();
    let kind = if ft.is_dir() {
      EntryKind::Dir
    } else if ft.is_symlink() {
      EntryKind::Symlink
    } else {
      EntryKind::File
    };
    out.push(Entry {
      pkg_rel: rel,
      abs_src: entry.path().to_path_buf(),
      kind,
    });
  }
  out.sort_by(|a, b| a.pkg_rel.cmp(&b.pkg_rel));
  Ok(out)
}

// ---------------------------------------------------------------- plan

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
  Mkdir {
    dst: PathBuf,
  },
  Fold {
    src_dir: PathBuf,
    dst: PathBuf,
  },
  Link {
    src: PathBuf,
    dst: PathBuf,
  },
  /// Remove a rustow/stow-owned symlink at `dst` pointing at a directory;
  /// replaced by a real directory whose contents are re-linked individually.
  Unfold {
    dst: PathBuf,
  },
  /// Replace a dangling or stale link at `dst` that points into this package.
  Repair {
    src: PathBuf,
    dst: PathBuf,
  },
  /// Already linked — nothing to do.
  SkipIdempotent {
    dst: PathBuf,
  },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictReason {
  /// A real file already occupies the target.
  Exists,
  /// Target is a directory but the package wants a file (or vice versa).
  DirFileMismatch,
  /// Target is a symlink to somewhere outside the source dir.
  ExternalSymlink,
  /// Target is a symlink into another package.
  WrongPackage,
  /// Target is a dangling symlink not pointing into this package.
  DanglingExternal,
  /// Two packages claim the same target in the same run.
  CrossPackage,
}

#[derive(Debug, Clone)]
pub struct Conflict {
  pub dst: PathBuf,
  pub reason: ConflictReason,
  pub src: Option<PathBuf>,
  /// Where the existing file/link points to or is, for messages.
  pub detail: Option<PathBuf>,
}

impl std::fmt::Display for Conflict {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let what = match self.reason {
      ConflictReason::Exists => "target exists",
      ConflictReason::DirFileMismatch => "file/directory mismatch",
      ConflictReason::ExternalSymlink => "symlink points outside the source tree",
      ConflictReason::WrongPackage => "symlink points into another package",
      ConflictReason::DanglingExternal => "dangling symlink not owned by rustow",
      ConflictReason::CrossPackage => "claimed by multiple packages",
    };
    write!(f, "{} ({})", self.dst.display(), what)
  }
}

#[derive(Debug, Default)]
pub struct Plan {
  pub actions: Vec<Action>,
  pub conflicts: Vec<Conflict>,
}

/// Lexical normalization: resolves `.` and `..` components without touching the fs.
pub fn lexical_normalize(p: &Path) -> PathBuf {
  let mut out = PathBuf::new();
  for comp in p.components() {
    match comp {
      std::path::Component::CurDir => {}
      std::path::Component::ParentDir => {
        out.pop();
      }
      c => out.push(c.as_os_str()),
    }
  }
  out
}

fn resolve_symlink(dst: &Path) -> Option<PathBuf> {
  let raw = fs::read_link(dst).ok()?;
  let abs = if raw.is_absolute() {
    raw
  } else {
    dst.parent().unwrap_or(Path::new(".")).join(raw)
  };
  Some(lexical_normalize(&abs))
}

enum Probe {
  Missing,
  RealDir,
  RealFile,
  Symlink(Option<PathBuf>),
}

fn dst_probe(dst: &Path) -> Probe {
  match fs::symlink_metadata(dst) {
    Ok(meta) => {
      if meta.file_type().is_symlink() {
        Probe::Symlink(resolve_symlink(dst))
      } else if meta.is_dir() {
        Probe::RealDir
      } else {
        Probe::RealFile
      }
    }
    Err(_) => Probe::Missing,
  }
}

pub struct Planner<'a> {
  pub source: &'a Path,
  pub target: &'a Path,
  pub pkg: &'a str,
  pub fold: bool,
  pub ignore: &'a Ignore,
  /// Fold destinations claimed by another package in this run.
  block_fold: HashSet<PathBuf>,
  /// Paths unfolded this run; paths at or below them probe as Missing.
  unfolded: Vec<PathBuf>,
  /// Dir destinations claimed by this package — the foreign re-link must
  /// not fold one of these (both packages would get their own link).
  own_dir_claims: HashSet<PathBuf>,
  /// Destinations minted by the foreign re-link; own entries claiming them are cross-package collisions.
  claimed: HashSet<PathBuf>,
  pub actions: Vec<Action>,
  pub conflicts: Vec<Conflict>,
}

impl<'a> Planner<'a> {
  pub fn new(
    source: &'a Path,
    target: &'a Path,
    pkg: &'a str,
    fold: bool,
    ignore: &'a Ignore,
    block_fold: HashSet<PathBuf>,
    entries_dirs: Vec<PathBuf>,
  ) -> Self {
    let own_dir_claims: HashSet<PathBuf> = entries_dirs.into_iter().collect();
    Self {
      source,
      target,
      pkg,
      fold,
      ignore,
      block_fold,
      unfolded: Vec::new(),
      own_dir_claims,
      claimed: HashSet::new(),
      actions: Vec::new(),
      conflicts: Vec::new(),
    }
  }

  fn pkg_root(&self) -> PathBuf {
    self.source.join(self.pkg)
  }

  fn probe(&self, dst: &Path) -> Probe {
    if self.is_under_unfolded(dst) {
      return Probe::Missing;
    }
    dst_probe(dst)
  }

  fn is_under_unfolded(&self, p: &Path) -> bool {
    self.unfolded.iter().any(|u| p.starts_with(u))
  }

  fn mark(&mut self, dst: &Path) {
    self.claimed.insert(dst.to_path_buf());
  }

  pub fn run(mut self, entries: &[Entry]) -> Plan {
    // only top-level entries here; directories plan their own children recursively
    for entry in entries {
      if entry.pkg_rel.components().count() != 1 {
        continue;
      }
      let dst = self.target.join(&entry.pkg_rel);
      match entry.kind {
        EntryKind::Dir => self.plan_dir(&entry.abs_src, &entry.pkg_rel, &dst),
        EntryKind::File | EntryKind::Symlink => {
          self.plan_leaf(&entry.abs_src, &entry.pkg_rel, &dst)
        }
      }
    }
    Plan {
      actions: self.actions,
      conflicts: self.conflicts,
    }
  }

  /// A directory folds only over a subtree free of ignored paths (e.g. nested
  /// .git); otherwise we descend and link per-file so ignored data is never
  /// exposed through a symlink.
  fn clean_subtree(&self, src_dir: &Path, pkg_rel: &Path) -> bool {
    for item in WalkDir::new(src_dir)
      .follow_links(false)
      .into_iter()
      .flatten()
    {
      let rel = match item.path().strip_prefix(src_dir) {
        Ok(p) if p.as_os_str().is_empty() => continue,
        Ok(p) => pkg_rel.join(p),
        Err(_) => continue,
      };
      if self.ignore.is_ignored(&rel) {
        return false;
      }
    }
    true
  }

  fn plan_dir(&mut self, src_dir: &Path, pkg_rel: &Path, dst: &Path) {
    match self.probe(dst) {
      Probe::Missing => {
        if self.fold
          && !self.block_fold.contains(dst)
          && !self.claimed.contains(dst)
          && self.has_contents(src_dir)
          && self.clean_subtree(src_dir, pkg_rel)
        {
          self.actions.push(Action::Fold {
            src_dir: src_dir.to_path_buf(),
            dst: dst.to_path_buf(),
          });
          self.mark(dst);
          return;
        }
        self.actions.push(Action::Mkdir {
          dst: dst.to_path_buf(),
        });
        self.mark(dst);
        self.plan_children(src_dir, pkg_rel, dst);
      }
      Probe::RealDir => {
        // real dir in the way — no fold here, recurse (deeper dirs may still fold)
        self.plan_children(src_dir, pkg_rel, dst);
      }
      Probe::Symlink(Some(resolved)) => {
        let own_pkg_root = self.pkg_root();
        let src_norm = lexical_normalize(src_dir);
        if resolved == src_norm {
          // re-stow of the same package where the fold is already in place
          self.actions.push(Action::SkipIdempotent {
            dst: dst.to_path_buf(),
          });
          return;
        }
        if resolved.starts_with(&own_pkg_root) {
          // points into this package but at a different directory — stale, refold
          self.actions.push(Action::Fold {
            src_dir: src_dir.to_path_buf(),
            dst: dst.to_path_buf(),
          });
          return;
        }
        if resolved.starts_with(self.source) {
          // folded dir owned by another package — unfold it
          self.actions.push(Action::Unfold {
            dst: dst.to_path_buf(),
          });
          self.unfolded.push(dst.to_path_buf());
          self.actions.push(Action::Mkdir {
            dst: dst.to_path_buf(),
          });
          self.relink_foreign_subtree(&resolved, dst);
          self.plan_children(src_dir, pkg_rel, dst);
          return;
        }
        self.conflicts.push(Conflict {
          dst: dst.to_path_buf(),
          reason: ConflictReason::ExternalSymlink,
          src: Some(src_dir.to_path_buf()),
          detail: Some(resolved),
        });
      }
      Probe::Symlink(None) => {
        self.conflicts.push(Conflict {
          dst: dst.to_path_buf(),
          reason: ConflictReason::DanglingExternal,
          src: Some(src_dir.to_path_buf()),
          detail: None,
        });
      }
      Probe::RealFile => {
        self.conflicts.push(Conflict {
          dst: dst.to_path_buf(),
          reason: ConflictReason::DirFileMismatch,
          src: Some(src_dir.to_path_buf()),
          detail: None,
        });
      }
    }
  }

  fn has_contents(&self, src_dir: &Path) -> bool {
    fs::read_dir(src_dir)
      .map(|mut it| it.next().is_some())
      .unwrap_or(false)
  }

  /// Re-link the subtree that used to be hidden behind a folded symlink at
  /// `foreign_dir` into the new real directory `dst` (made by Unfold + Mkdir).
  fn relink_foreign_subtree(&mut self, foreign_dir: &Path, dst: &Path) {
    let rel = match foreign_dir.strip_prefix(self.source) {
      Ok(r) => r.to_path_buf(),
      Err(_) => return,
    };
    let foreign_pkg = match rel.components().next() {
      Some(std::path::Component::Normal(n)) => n.to_string_lossy().into_owned(),
      _ => return,
    };
    let pkg_root = self.source.join(&foreign_pkg);
    // prefix of the folded dir inside its package (e.g. ".config/shared")
    let prefix = match foreign_dir.strip_prefix(&pkg_root) {
      Ok(p) => p.to_path_buf(),
      Err(_) => return,
    };

    let mut stack = vec![foreign_dir.to_path_buf()];
    while let Some(abs_dir) = stack.pop() {
      let children = match fs::read_dir(&abs_dir) {
        Ok(rd) => rd,
        Err(_) => continue, // foreign dir is gone — nothing to re-link
      };
      let mut children: Vec<_> = children.flatten().collect();
      children.sort_by_key(|c| c.file_name());
      for child in children {
        let Ok(subtree_rel) = child.path().strip_prefix(foreign_dir).map(PathBuf::from) else {
          continue;
        };
        let full_pkg_rel = prefix.join(&subtree_rel);
        if self.ignore.is_ignored(&full_pkg_rel) {
          continue;
        }
        let child_dst = dst.join(&subtree_rel);
        let child_abs = child.path();
        let child_is_dir = child.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if child_is_dir {
          if self.fold
            && !self.block_fold.contains(&child_dst)
            && !self.own_dir_claims.contains(&child_dst)
            && !self.claimed.contains(&child_dst)
            && self.has_contents(&child_abs)
            && self.clean_subtree(&child_abs, &full_pkg_rel)
          {
            self.actions.push(Action::Fold {
              src_dir: child_abs,
              dst: child_dst.clone(),
            });
            self.mark(&child_dst);
          } else {
            self.actions.push(Action::Mkdir {
              dst: child_dst.clone(),
            });
            self.mark(&child_dst);
            stack.push(child_abs);
          }
        } else {
          self.actions.push(Action::Link {
            src: child_abs,
            dst: child_dst.clone(),
          });
          self.mark(&child_dst);
        }
      }
    }
  }

  fn plan_children(&mut self, src_dir: &Path, pkg_rel: &Path, dst: &Path) {
    let mut children: Vec<_> = match fs::read_dir(src_dir) {
      Ok(rd) => rd.flatten().collect(),
      Err(_) => return,
    };
    children.sort_by_key(|c| c.file_name());
    for child in children {
      let name = child.file_name();
      let child_pkg_rel = pkg_rel.join(&name);
      if self.ignore.is_ignored(&child_pkg_rel) {
        continue;
      }
      let child_abs = child.path();
      let child_dst = dst.join(&name);
      // classify WITHOUT following symlinks — a package symlink is a leaf
      let is_dir = child.file_type().map(|t| t.is_dir()).unwrap_or(false);
      if is_dir {
        self.plan_dir(&child_abs, &child_pkg_rel, &child_dst);
      } else {
        self.plan_leaf(&child_abs, &child_pkg_rel, &child_dst);
      }
    }
  }

  fn plan_leaf(&mut self, src: &Path, _pkg_rel: &Path, dst: &Path) {
    if self.claimed.contains(dst) {
      self.conflicts.push(Conflict {
        dst: dst.to_path_buf(),
        reason: ConflictReason::CrossPackage,
        src: Some(src.to_path_buf()),
        detail: None,
      });
      return;
    }
    match self.probe(dst) {
      Probe::Missing => {
        self.actions.push(Action::Link {
          src: src.to_path_buf(),
          dst: dst.to_path_buf(),
        });
        self.mark(dst);
      }
      Probe::RealDir => self.conflicts.push(Conflict {
        dst: dst.to_path_buf(),
        reason: ConflictReason::DirFileMismatch,
        src: Some(src.to_path_buf()),
        detail: None,
      }),
      Probe::RealFile => self.conflicts.push(Conflict {
        dst: dst.to_path_buf(),
        reason: ConflictReason::Exists,
        src: Some(src.to_path_buf()),
        detail: None,
      }),
      Probe::Symlink(maybe_target) => {
        let src_abs = lexical_normalize(src);
        match &maybe_target {
          Some(rl) if *rl == src_abs => {
            self.actions.push(Action::SkipIdempotent {
              dst: dst.to_path_buf(),
            });
          }
          Some(rl) if rl.starts_with(self.pkg_root()) => {
            self.actions.push(Action::Repair {
              src: src.to_path_buf(),
              dst: dst.to_path_buf(),
            });
          }
          Some(rl) if rl.starts_with(self.source) => {
            self.conflicts.push(Conflict {
              dst: dst.to_path_buf(),
              reason: ConflictReason::WrongPackage,
              src: Some(src.to_path_buf()),
              detail: Some(rl.clone()),
            });
          }
          Some(rl) if rl.exists() => {
            self.conflicts.push(Conflict {
              dst: dst.to_path_buf(),
              reason: ConflictReason::ExternalSymlink,
              src: Some(src.to_path_buf()),
              detail: Some(rl.clone()),
            });
          }
          Some(rl) => {
            self.conflicts.push(Conflict {
              dst: dst.to_path_buf(),
              reason: ConflictReason::DanglingExternal,
              src: Some(src.to_path_buf()),
              detail: Some(rl.clone()),
            });
          }
          None => self.conflicts.push(Conflict {
            dst: dst.to_path_buf(),
            reason: ConflictReason::DanglingExternal,
            src: Some(src.to_path_buf()),
            detail: None,
          }),
        }
      }
    }
  }
}

/// A directory fold is blocked when another package claims any target at or
/// below the fold destination in this run.
fn compute_blocked_folds(
  target: &Path,
  pkg_entries: &HashMap<String, Vec<Entry>>,
) -> HashSet<PathBuf> {
  let dir_claims: Vec<(PathBuf, &str)> = pkg_entries
    .iter()
    .flat_map(|(pkg, entries)| {
      entries
        .iter()
        .filter(|e| e.kind == EntryKind::Dir)
        .map(|e| (target.join(&e.pkg_rel), pkg.as_str()))
    })
    .collect();
  let all_claims: Vec<(PathBuf, &str)> = pkg_entries
    .iter()
    .flat_map(|(pkg, entries)| {
      entries
        .iter()
        .map(|e| (target.join(&e.pkg_rel), pkg.as_str()))
    })
    .collect();

  let mut blocked = HashSet::new();
  for (dst, pkg) in &dir_claims {
    let blocked_by_other = all_claims
      .iter()
      .any(|(p, other)| other != pkg && (p == dst || p.starts_with(dst)));
    if blocked_by_other {
      blocked.insert(dst.clone());
    }
  }
  blocked
}

/// Cross-package same-target collisions across the whole run. Every claimant
/// after the first (packages sorted by name) gets a `CrossPackage` conflict.
/// Dir/dir claims merge and are not conflicts.
pub fn cross_package_conflicts(
  target: &Path,
  pkg_entries: &HashMap<String, Vec<Entry>>,
) -> HashMap<String, Vec<Conflict>> {
  type Claim<'a> = (&'a String, EntryKind, Option<PathBuf>);
  let mut claims: HashMap<PathBuf, Vec<Claim<'_>>> = HashMap::new();
  let mut pkgs: Vec<&String> = pkg_entries.keys().collect();
  pkgs.sort();
  for pkg in &pkgs {
    for e in &pkg_entries[*pkg] {
      let dst = target.join(&e.pkg_rel);
      let src = match e.kind {
        EntryKind::Dir => None,
        _ => Some(e.abs_src.clone()),
      };
      claims.entry(dst).or_default().push((pkg, e.kind, src));
    }
  }
  let mut out: HashMap<String, Vec<Conflict>> = HashMap::new();
  for (dst, claimants) in claims {
    if claimants.len() < 2 {
      continue;
    }
    let all_dirs = claimants.iter().all(|(_, k, _)| *k == EntryKind::Dir);
    if all_dirs {
      continue;
    }
    for (i, (pkg, _, src)) in claimants.iter().enumerate() {
      if i == 0 {
        continue;
      }
      out.entry(pkg.to_string()).or_default().push(Conflict {
        dst: dst.clone(),
        reason: ConflictReason::CrossPackage,
        src: src.clone(),
        detail: None,
      });
    }
  }
  out
}

pub struct LinkInput<'a> {
  pub source: &'a Path,
  pub target: &'a Path,
  pub pkgs: &'a [String],
  pub fold: bool,
  pub ignore: &'a Ignore,
}

/// Plan link actions for all packages up front, so the caller can abort the
/// whole run on any conflict before touching the filesystem.
fn validate_pkg(source: &Path, pkg: &str) -> Result<()> {
  if pkg.is_empty()
    || pkg == "."
    || pkg == ".."
    || pkg.contains('/')
    || pkg.contains('\\')
    || !source.join(pkg).is_dir()
  {
    return Err(Error::Message(format!(
      "invalid package: {pkg:?} (must be a direct, existing subdirectory of {})",
      source.display()
    )));
  }
  Ok(())
}

pub fn plan_link_all(input: &LinkInput<'_>) -> Result<Vec<Plan>> {
  let mut pkg_entries: HashMap<String, Vec<Entry>> = HashMap::new();
  for pkg in input.pkgs {
    validate_pkg(input.source, pkg)?;
    pkg_entries.insert(pkg.clone(), scan_package(input.source, pkg, input.ignore)?);
  }
  let blocked = compute_blocked_folds(input.target, &pkg_entries);
  let mut cross = cross_package_conflicts(input.target, &pkg_entries);

  let mut plans = Vec::with_capacity(input.pkgs.len());
  for pkg in input.pkgs {
    let entries = pkg_entries.get(pkg).cloned().unwrap_or_default();
    let entries_dirs: Vec<PathBuf> = entries
      .iter()
      .filter(|e| e.kind == EntryKind::Dir)
      .map(|e| input.target.join(&e.pkg_rel))
      .collect();
    let planner = Planner::new(
      input.source,
      input.target,
      pkg,
      input.fold,
      input.ignore,
      blocked.clone(),
      entries_dirs,
    );
    let mut plan = planner.run(&entries);
    if let Some(mut extra) = cross.remove(pkg) {
      plan.conflicts.append(&mut extra);
    }
    plans.push(plan);
  }
  Ok(plans)
}

// ---------------------------------------------------------------- apply

/// How to resolve conflicts found during planning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConflictResolution {
  /// Skip linking the conflicting entry (leave target alone).
  Skip,
  /// Delete target; symlink package version.
  Overwrite,
  /// Move target's content into the package; symlink.
  Adopt,
  /// Leave everything and abort (nothing is applied).
  Abort,
  /// Ask the user (TTY); non-TTY escalates to Abort.
  #[default]
  Prompt,
}

/// How a prompt for a single conflict turned out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptOutcome {
  Skip,
  Overwrite,
  Adopt,
  /// Merge happened interactively; treat like overwrite afterwards.
  Merge,
  Abort,
}

/// Non-interactive conflict decision source; implementations supply the TTY prompt.
pub trait ConflictResolver {
  fn resolve(&mut self, conflict: &Conflict) -> Result<PromptOutcome>;
}

/// Always resolves without asking (for `-f`/`--adopt`/`on_conflict=...`).
pub struct FixedResolver(pub ConflictResolution);

impl ConflictResolver for FixedResolver {
  fn resolve(&mut self, _conflict: &Conflict) -> Result<PromptOutcome> {
    Ok(match self.0 {
      ConflictResolution::Skip => PromptOutcome::Skip,
      ConflictResolution::Overwrite => PromptOutcome::Overwrite,
      ConflictResolution::Adopt => PromptOutcome::Adopt,
      ConflictResolution::Abort | ConflictResolution::Prompt => PromptOutcome::Abort,
    })
  }
}

/// Aborting resolver used when not a TTY without -f/--adopt (default safe path).
pub struct AbortResolver;

impl ConflictResolver for AbortResolver {
  fn resolve(&mut self, _conflict: &Conflict) -> Result<PromptOutcome> {
    Ok(PromptOutcome::Abort)
  }
}

#[derive(Debug, Default, Clone)]
pub struct ApplyOutcome {
  pub linked: usize,
  pub repaired: usize,
  pub skipped: usize,
  pub dry_run: bool,
  pub warnings: Vec<String>,
}

/// Apply link plans. All conflicts are resolved via `resolver` (or skipped
/// per resolution). If any conflict resolves to Abort, nothing is applied.
/// Action application is idempotent. In `dry_run`, no resolution or fs change
/// happens — conflicts are only counted and reported.
pub fn apply_plans(
  plans: &[Plan],
  source: &Path,
  resolver: &mut dyn ConflictResolver,
  dry_run: bool,
) -> Result<ApplyOutcome> {
  // dry-run: never resolve, never touch the fs — just report the plan
  if dry_run {
    let mut out = ApplyOutcome {
      dry_run: true,
      ..ApplyOutcome::default()
    };
    let mut conflicts = 0usize;
    for plan in plans {
      conflicts += plan.conflicts.len();
      for a in &plan.actions {
        match a {
          Action::Link { .. } | Action::Fold { .. } => out.linked += 1,
          Action::Repair { .. } => out.repaired += 1,
          _ => {}
        }
      }
    }
    if conflicts > 0 {
      out.warnings.push(format!(
        "{conflicts} conflict(s) unresolved (dry run — nothing was changed)"
      ));
      out.skipped = conflicts;
    }
    return Ok(out);
  }

  // 1. conflicts first — two-phase
  let mut outcomes: Vec<(Conflict, PromptOutcome)> = Vec::new();
  for plan in plans {
    for c in &plan.conflicts {
      outcomes.push((c.clone(), resolver.resolve(c)?));
    }
  }
  if outcomes.iter().any(|(_, o)| *o == PromptOutcome::Abort) {
    let report = render_conflict_report(&outcomes);
    return Err(Error::Conflicts(report));
  }

  // 2. rewrite conflict actions into fs ops we can apply
  let mut actions: Vec<Action> = Vec::new();
  let mut adopted: Vec<(PathBuf, PathBuf)> = Vec::new(); // (target, package_src)
  let mut overwritten: Vec<PathBuf> = Vec::new();
  let mut skip_dsts: HashSet<PathBuf> = HashSet::new();
  let mut fixup_dsts: HashSet<PathBuf> = HashSet::new();
  for (c, outcome) in outcomes {
    let src = c.src.clone();
    match outcome {
      PromptOutcome::Skip => {
        skip_dsts.insert(c.dst.clone());
      }
      PromptOutcome::Overwrite | PromptOutcome::Merge => {
        overwritten.push(c.dst.clone());
        fixup_dsts.insert(c.dst.clone());
        if let Some(src) = src {
          actions.push(Action::Link { src, dst: c.dst });
        }
      }
      PromptOutcome::Adopt => {
        fixup_dsts.insert(c.dst.clone());
        if let Some(src) = src {
          adopted.push((c.dst.clone(), src.clone()));
          actions.push(Action::Link { src, dst: c.dst });
        }
      }
      PromptOutcome::Abort => unreachable!("checked above"),
    }
  }
  let mut suppressed = 0usize;
  for plan in plans {
    for a in &plan.actions {
      match a {
        Action::SkipIdempotent { .. } => {}
        Action::Link { dst, .. } | Action::Fold { dst, .. } | Action::Repair { dst, .. }
          if skip_dsts.contains(dst) || fixup_dsts.contains(dst) =>
        {
          suppressed += 1;
        }
        other => actions.push(other.clone()),
      }
    }
  }

  let mut out_warnings: Vec<String> = Vec::new();
  if suppressed > 0 {
    out_warnings.push(format!(
      "suppressed {suppressed} duplicate link action(s) at targets claimed by another package"
    ));
  }
  let mut out = ApplyOutcome {
    dry_run,
    ..ApplyOutcome::default()
  };

  // 3. adopt/overwrite fixups before linking
  let mut drop_actions: HashSet<PathBuf> = HashSet::new();
  for (target, src) in &adopted {
    let is_regular_file = fs::symlink_metadata(target)
      .map(|m| m.file_type().is_file())
      .unwrap_or(false);
    if !is_regular_file {
      out_warnings.push(format!(
        "cannot adopt {}: not a regular file — left in place",
        target.display()
      ));
      skip_dsts.insert(target.clone());
      drop_actions.insert(target.clone());
      continue;
    }
    if let Some(parent) = src.parent() {
      fs::create_dir_all(parent).map_err(Error::io(parent))?;
    }
    if let Err(rename_err) = fs::rename(target, src) {
      // cross-device: copy then remove (rename cannot cross filesystems)
      fs::copy(target, src).map_err(|e| {
        Error::Message(format!(
          "adopt: failed to move {} into the package ({rename_err}; copy also failed: {e})",
          target.display()
        ))
      })?;
      fs::remove_file(target).map_err(Error::io(target))?;
    }
  }
  for target in &overwritten {
    if target.is_dir() && !target.is_symlink() {
      // never delete a real directory tree — leave it alone and skip the link
      out_warnings.push(format!(
        "refusing to overwrite real directory {} — left untouched",
        target.display()
      ));
      skip_dsts.insert(target.clone());
      drop_actions.insert(target.clone());
      continue;
    } else if target.exists() || target.is_symlink() {
      fs::remove_file(target).map_err(Error::io(target))?;
    }
  }

  // 4. execute actions in order (skipping adopts that were rejected)
  for action in actions.into_iter().filter(|a| match a {
    Action::Link { dst, .. } => !drop_actions.contains(dst),
    _ => true,
  }) {
    let action = &action;
    match action {
      Action::Mkdir { dst } => {
        fs::create_dir_all(dst).map_err(Error::io(dst))?;
      }
      Action::Fold { src_dir, dst } => {
        // a Fold aimed at an existing owned symlink is a stale fold from an
        // earlier run with a renamed package dir — replace it
        if dst.is_symlink() && is_owned_link(dst, source) {
          fs::remove_file(dst).map_err(Error::io(dst))?;
        }
        apply_link(src_dir, dst)?;
        out.linked += 1;
      }
      Action::Link { src: src_dir, dst } => {
        apply_link(src_dir, dst)?;
        out.linked += 1;
      }
      Action::Repair { src, dst } => {
        if dst.is_symlink() {
          fs::remove_file(dst).map_err(Error::io(dst))?;
        }
        apply_link(src, dst)?;
        out.repaired += 1;
      }
      Action::Unfold { dst } => {
        if dst.is_symlink() && is_owned_link(dst, source) {
          fs::remove_file(dst).map_err(Error::io(dst))?;
        } else if !dst.exists() || dst.is_dir() {
          // already a real dir (or missing) — nothing to do
        } else {
          return Err(Error::Message(format!(
            "cannot unfold {}: not an owned symlink",
            dst.display()
          )));
        }
      }
      Action::SkipIdempotent { .. } => {}
    }
  }
  out.skipped = skip_dsts.len();
  out.warnings = out_warnings;
  Ok(out)
}

fn apply_link(src: &Path, dst: &Path) -> Result<()> {
  if let Some(parent) = dst.parent() {
    fs::create_dir_all(parent).map_err(Error::io(parent))?;
  }
  match fs::symlink_metadata(dst) {
    Ok(meta) => {
      if meta.file_type().is_symlink()
        && fs::read_link(dst)
          .ok()
          .map(|r| absolutize(&r, parent_of(dst)))
          .as_deref()
          == Some(src)
      {
        return Ok(()); // already correct
      }
      // stale plan — the world changed between plan and apply
      Err(Error::Message(format!(
        "{} appeared while linking (plan was stale); re-run rustow",
        dst.display()
      )))
    }
    Err(_) => {
      platform::symlink(src, dst)?;
      Ok(())
    }
  }
}

fn parent_of(p: &Path) -> &Path {
  p.parent().unwrap_or(Path::new("."))
}

fn absolutize(raw: &Path, base: &Path) -> PathBuf {
  if raw.is_absolute() {
    lexical_normalize(raw)
  } else {
    lexical_normalize(&base.join(raw))
  }
}

/// A symlink is rustow/stow-owned when its resolved target lives inside the source tree.
fn is_owned_link(dst: &Path, source: &Path) -> bool {
  fs::read_link(dst)
    .map(|r| absolutize(&r, parent_of(dst)).starts_with(source))
    .unwrap_or(false)
}

fn render_conflict_report(outcomes: &[(Conflict, PromptOutcome)]) -> String {
  let mut out = String::from("rustow: conflicts detected — nothing was changed:\n");
  for (c, _) in outcomes {
    out.push_str(&format!("  {c}\n"));
    if let Some(detail) = &c.detail {
      out.push_str(&format!("      existing target: {}\n", detail.display()));
    }
  }
  out
}

// ---------------------------------------------------------------- unlink

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnlinkAction {
  Unlink { dst: PathBuf },
}

#[derive(Debug, Default)]
pub struct UnlinkPlan {
  pub actions: Vec<UnlinkAction>,
  /// Real dirs traversed during the unlink walk — refold candidates (deepest first).
  pub candidate_dirs: std::collections::HashSet<PathBuf>,
  /// Informational: entries found on disk that were NOT ours (left alone).
  pub foreign_skipped: Vec<PathBuf>,
}

/// Plan unlinking of `pkgs`: removes symlinks pointing into each package.
/// Never touches anything not owned by a package. Refold of now link-only
/// dirs happens at apply time (they are evaluated against the post-unlink
/// filesystem).
pub fn plan_unlink(
  source: &Path,
  target: &Path,
  pkgs: &[String],
  ignore: &Ignore,
) -> Result<UnlinkPlan> {
  let mut plan = UnlinkPlan::default();
  for pkg in pkgs {
    validate_pkg(source, pkg)?;
    let pkg_root = source.join(pkg);
    if !pkg_root.is_dir() {
      return Err(Error::PackageNotFound(pkg.clone()));
    }
    walk_unlink(
      source,
      target,
      &pkg_root,
      &pkg_root,
      Path::new(""),
      ignore,
      &mut plan,
    )?;
  }
  Ok(plan)
}

fn walk_unlink(
  _source: &Path,
  target: &Path,
  pkg_root: &Path,
  subdir_abs: &Path,
  subdir_rel: &Path,
  ignore: &Ignore,
  plan: &mut UnlinkPlan,
) -> Result<()> {
  let children: Vec<PathBuf> = match fs::read_dir(subdir_abs) {
    Ok(rd) => rd.flatten().map(|e| e.path()).collect(),
    Err(_) => return Ok(()),
  };
  let mut children = children;
  children.sort();

  for child in children {
    let name = match child.file_name() {
      Some(n) => n,
      None => continue,
    };
    let pkg_rel = subdir_rel.join(name);
    if ignore.is_ignored(&pkg_rel) {
      continue;
    }
    let dst = target.join(&pkg_rel);
    let meta = fs::symlink_metadata(&dst).ok();
    match (child.is_dir(), meta) {
      (true, Some(m)) if m.file_type().is_symlink() => {
        if link_points_into(&dst, pkg_root) {
          plan.actions.push(UnlinkAction::Unlink { dst: dst.clone() });
          plan
            .candidate_dirs
            .insert(dst.parent().unwrap_or(target).to_path_buf());
        } else {
          plan.foreign_skipped.push(dst);
        }
      }
      (true, Some(m)) if m.is_dir() => {
        plan.candidate_dirs.insert(dst.clone());
        let child_abs = child.clone();
        walk_unlink(
          _source, target, pkg_root, &child_abs, &pkg_rel, ignore, plan,
        )?;
      }
      (false, Some(m)) if m.file_type().is_symlink() => {
        if link_points_into(&dst, pkg_root) {
          plan.actions.push(UnlinkAction::Unlink { dst: dst.clone() });
          plan
            .candidate_dirs
            .insert(dst.parent().unwrap_or(target).to_path_buf());
        } else {
          plan.foreign_skipped.push(dst);
        }
      }
      _ => {}
    }
  }
  Ok(())
}

fn link_points_into(dst: &Path, pkg_root: &Path) -> bool {
  fs::read_link(dst)
    .ok()
    .map(|r| absolutize(&r, parent_of(dst)).starts_with(pkg_root))
    .unwrap_or(false)
}

/// If `dir` is a real dir whose children are ALL symlinks resolving into one
/// package's matching subtree (`source/<pkg>/<dir-rel>`), return that subtree.
/// The target root itself is never refolded.
pub fn refolded_dir(
  dir: &Path,
  target: &Path,
  source: &Path,
  ignore: &crate::ignore::Ignore,
) -> Option<PathBuf> {
  if dir == target || !dir.starts_with(target) {
    return None;
  }
  let rel = dir.strip_prefix(target).ok()?.to_path_buf();
  if rel.as_os_str().is_empty() {
    return None;
  }
  let meta = fs::symlink_metadata(dir).ok()?;
  if !meta.is_dir() || meta.file_type().is_symlink() {
    return None;
  }
  let children: Vec<PathBuf> = fs::read_dir(dir)
    .ok()?
    .flatten()
    .map(|e| e.path())
    .collect();
  if children.is_empty() {
    return None; // empty — handled by rmdir
  }
  let mut common: Option<PathBuf> = None;
  for child in &children {
    let meta = fs::symlink_metadata(child).ok()?;
    if !meta.file_type().is_symlink() {
      return None;
    }
    let rl = resolve_symlink(child)?;
    // package subtree corresponding to `dir`: the deepest ancestor of rl ending with rel
    let pkg_dir = rl.ancestors().find(|a| a.ends_with(&rel))?.to_path_buf();
    // must be a package subtree: source/<pkg>/<rel> with <pkg> a direct child of source
    if !pkg_dir.starts_with(source) || !pkg_dir.is_dir() {
      return None;
    }
    let from_source = pkg_dir.strip_prefix(source).ok()?;
    let mut comps = from_source.components();
    let Some(std::path::Component::Normal(pkg_name)) = comps.next() else {
      return None;
    };
    let rest: PathBuf = comps.collect();
    if rest != rel || !source.join(pkg_name).is_dir() {
      return None; // not a <source>/<pkg>/<rel> shape — refuse to refold anything else
    }
    match &common {
      None => common = Some(pkg_dir),
      Some(c) if *c == pkg_dir => {}
      Some(_) => return None,
    }
  }
  let package_dir = common?;
  // never refold a subtree containing ignored paths (e.g. nested .git) —
  // the spec's "ignored data is never exposed through a symlink" covers unlink too
  for item in WalkDir::new(&package_dir)
    .follow_links(false)
    .into_iter()
    .flatten()
  {
    let sub = match item.path().strip_prefix(&package_dir) {
      Ok(p) if p.as_os_str().is_empty() => continue,
      Ok(p) => p,
      Err(_) => continue,
    };
    if ignore.is_ignored(&rel.join(sub)) {
      return None;
    }
  }
  Some(package_dir)
}

fn try_refold(
  dir: &Path,
  target: &Path,
  source: &Path,
  ignore: &crate::ignore::Ignore,
) -> Result<bool> {
  let Some(package_dir) = refolded_dir(dir, target, source, ignore) else {
    return Ok(false);
  };
  for child in fs::read_dir(dir).map_err(Error::io(dir))?.flatten() {
    let p = child.path();
    if p.is_symlink() {
      fs::remove_file(&p).map_err(Error::io(&p))?;
    } else {
      // foreign file appeared — don't touch
      return Ok(false);
    }
  }
  fs::remove_dir(dir).map_err(Error::io(dir))?;
  platform::symlink(&package_dir, dir)?;
  Ok(true)
}

/// Apply an unlink plan: remove links, re-fold link-only dirs (post-unlink
/// filesystem), and remove dirs that became empty. Never the target root.
/// Returns the number of links removed (excluding refolds).
pub fn apply_unlink(
  plan: &UnlinkPlan,
  target: &Path,
  source: &Path,
  ignore: &crate::ignore::Ignore,
  dry_run: bool,
) -> Result<usize> {
  let mut removed = 0usize;

  if dry_run {
    return Ok(plan.actions.len());
  }

  for action in &plan.actions {
    let UnlinkAction::Unlink { dst } = action;
    if dst.is_symlink() {
      fs::remove_file(dst).map_err(Error::io(dst))?;
      removed += 1;
    }
    // dst no longer a symlink (changed since planning): skip, don't count
  }

  // re-fold link-only dirs, deepest first; a refold can make a parent foldable too
  let mut dirs: Vec<PathBuf> = plan.candidate_dirs.iter().cloned().collect();
  dirs.sort_by_key(|d| std::cmp::Reverse(d.components().count()));
  for dir in &dirs {
    try_refold(dir, target, source, ignore)?;
  }

  // remove dirs that became empty (walk deepest candidates + their parents)
  let mut seen = std::collections::HashSet::new();
  for dir in &dirs {
    try_rmdir_up(dir, target, &mut seen);
  }

  Ok(removed)
}

fn try_rmdir_up(dir: &Path, target: &Path, seen: &mut std::collections::HashSet<PathBuf>) {
  let mut current = Some(dir.to_path_buf());
  while let Some(d) = current {
    if d == *target || !d.starts_with(target) {
      break;
    }
    if !seen.insert(d.clone()) {
      break;
    }
    let empty = fs::read_dir(&d)
      .map(|mut r| r.next().is_none())
      .unwrap_or(false);
    if empty {
      let _ = fs::remove_dir(&d);
      current = d.parent().map(|p| p.to_path_buf());
    } else {
      break;
    }
  }
}
