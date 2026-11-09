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
        out.push(Entry { pkg_rel: rel, abs_src: entry.path().to_path_buf(), kind });
    }
    out.sort_by(|a, b| a.pkg_rel.cmp(&b.pkg_rel));
    Ok(out)
}

// ---------------------------------------------------------------- plan

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Mkdir { dst: PathBuf },
    Fold { src_dir: PathBuf, dst: PathBuf },
    Link { src: PathBuf, dst: PathBuf },
    /// Remove a rustow/stow-owned symlink at `dst` pointing at a directory;
    /// replaced by a real directory whose contents are re-linked individually.
    Unfold { dst: PathBuf },
    /// Replace a dangling or stale link at `dst` that points into this package.
    Repair { src: PathBuf, dst: PathBuf },
    /// Already linked — nothing to do.
    SkipIdempotent { dst: PathBuf },
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
        let own_dir_claims: HashSet<PathBuf> = entries_dirs
            .into_iter()
            .collect();
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
                EntryKind::File | EntryKind::Symlink => self.plan_leaf(&entry.abs_src, &entry.pkg_rel, &dst),
            }
        }
        Plan { actions: self.actions, conflicts: self.conflicts }
    }

    fn plan_dir(&mut self, src_dir: &Path, pkg_rel: &Path, dst: &Path) {
        match self.probe(dst) {
            Probe::Missing => {
                if self.fold
                    && !self.block_fold.contains(dst)
                    && !self.claimed.contains(dst)
                    && self.has_contents(src_dir)
                {
                    self.actions.push(Action::Fold { src_dir: src_dir.to_path_buf(), dst: dst.to_path_buf() });
                    self.mark(dst);
                    return;
                }
                self.actions.push(Action::Mkdir { dst: dst.to_path_buf() });
                self.mark(dst);
                self.plan_children(src_dir, pkg_rel, dst);
            }
            Probe::RealDir => {
                // real dir in the way — no fold here, recurse (deeper dirs may still fold)
                self.plan_children(src_dir, pkg_rel, dst);
            }
            Probe::Symlink(Some(resolved)) => {
                let own_pkg_root = self.pkg_root();
                let src_norm = lexical_normalize(&src_dir.to_path_buf());
                if resolved == src_norm {
                    // re-stow of the same package where the fold is already in place
                    self.actions.push(Action::SkipIdempotent { dst: dst.to_path_buf() });
                    return;
                }
                if resolved.starts_with(&own_pkg_root) {
                    // points into this package but at a different directory — stale, refold
                    self.actions.push(Action::Fold { src_dir: src_dir.to_path_buf(), dst: dst.to_path_buf() });
                    return;
                }
                if resolved.starts_with(self.source) {
                    // folded dir owned by another package — unfold it
                    self.actions.push(Action::Unfold { dst: dst.to_path_buf() });
                    self.unfolded.push(dst.to_path_buf());
                    self.actions.push(Action::Mkdir { dst: dst.to_path_buf() });
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
        fs::read_dir(src_dir).map(|mut it| it.next().is_some()).unwrap_or(false)
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
                let subtree_rel = child.path().strip_prefix(foreign_dir).unwrap().to_path_buf();
                let full_pkg_rel = prefix.join(&subtree_rel);
                if self.ignore.is_ignored(&full_pkg_rel) {
                    continue;
                }
                let child_dst = dst.join(&subtree_rel);
                let child_abs = child.path();
                if child_abs.is_dir() {
                    if self.fold
                        && !self.block_fold.contains(&child_dst)
                        && !self.own_dir_claims.contains(&child_dst)
                        && !self.claimed.contains(&child_dst)
                        && self.has_contents(&child_abs)
                    {
                        self.actions.push(Action::Fold { src_dir: child_abs, dst: child_dst.clone() });
                        self.mark(&child_dst);
                    } else {
                        self.actions.push(Action::Mkdir { dst: child_dst.clone() });
                        self.mark(&child_dst);
                        stack.push(child_abs);
                    }
                } else {
                    self.actions.push(Action::Link { src: child_abs, dst: child_dst.clone() });
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
            if child_abs.is_dir() {
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
                self.actions.push(Action::Link { src: src.to_path_buf(), dst: dst.to_path_buf() });
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
                let src_abs = lexical_normalize(&src.to_path_buf());
                match &maybe_target {
                    Some(rl) if *rl == src_abs => {
                        self.actions.push(Action::SkipIdempotent { dst: dst.to_path_buf() });
                    }
                    Some(rl) if rl.starts_with(&self.pkg_root()) => {
                        self.actions.push(Action::Repair { src: src.to_path_buf(), dst: dst.to_path_buf() });
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
            entries.iter().map(|e| (target.join(&e.pkg_rel), pkg.as_str()))
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
    let mut claims: HashMap<PathBuf, Vec<(&String, EntryKind, Option<PathBuf>)>> = HashMap::new();
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
pub fn plan_link_all(input: &LinkInput<'_>) -> Result<Vec<Plan>> {
    let mut pkg_entries: HashMap<String, Vec<Entry>> = HashMap::new();
    for pkg in input.pkgs {
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
        let planner = Planner::new(input.source, input.target, pkg, input.fold, input.ignore, blocked.clone(), entries_dirs);
        let mut plan = planner.run(&entries);
        if let Some(mut extra) = cross.remove(pkg) {
            plan.conflicts.append(&mut extra);
        }
        plans.push(plan);
    }
    Ok(plans)
}
