//! `rustow status` — per-package link health, table/short/JSON rendering.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{Error, Result};
use crate::ignore::Ignore;
use crate::link::{self, EntryKind, RootMappings, lexical_normalize, scan_package};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StatusKind {
  Linked,
  Partial,
  Unlinked,
  Conflict,
}

impl StatusKind {
  pub fn as_str(self) -> &'static str {
    match self {
      StatusKind::Linked => "linked",
      StatusKind::Partial => "partial",
      StatusKind::Unlinked => "unlinked",
      StatusKind::Conflict => "conflict",
    }
  }
}

#[derive(Debug, Clone, Serialize)]
pub struct EntryIssue {
  pub target: String,
  pub reason: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PackageStatus {
  pub name: String,
  pub status: StatusKind,
  pub total: usize,
  pub linked: usize,
  pub missing: usize,
  pub conflicts: usize,
  pub notes: Vec<String>,
  pub issues: Vec<EntryIssue>,
}

#[derive(Debug, Serialize)]
pub struct Summary {
  pub total: usize,
  pub linked: usize,
  pub partial: usize,
  pub unlinked: usize,
  pub conflict: usize,
}

#[derive(Debug, Serialize)]
pub struct JsonOutput {
  /// Schema version for machine consumers; bumped only on breaking shape changes (ADR 0005).
  pub version: u8,
  pub packages: Vec<PackageStatus>,
  pub summary: Summary,
}

fn resolve_link(dst: &Path) -> Option<PathBuf> {
  let raw = fs::read_link(dst).ok()?;
  let abs = if raw.is_absolute() {
    raw
  } else {
    dst.parent()?.join(raw)
  };
  Some(lexical_normalize(&abs))
}

/// Is `dst` a symlink resolving inside `pkg_root`?
fn is_pkg_link(dst: &Path, pkg_root: &Path) -> bool {
  resolve_link(dst)
    .map(|r| r.starts_with(pkg_root))
    .unwrap_or(false)
}

pub fn compute_package(
  source: &Path,
  target: &Path,
  pkg: &str,
  ignore: &Ignore,
  roots: &RootMappings,
) -> Result<PackageStatus> {
  let pkg_root = source.join(pkg);
  let root_map = roots.get(pkg);
  let mut entries = scan_package(source, pkg, ignore)?;
  // Root-mapped paths are managed at their absolute destinations, not in the
  // target tree. Withhold them from the ordinary walk exactly as the linker's
  // planner does, so a mapped file is not reported as "not linked" in $HOME.
  if let Some(mappings) = root_map {
    entries.retain(|e| {
      !mappings.keys().any(|key| {
        let rel = lexical_normalize(Path::new(key));
        e.pkg_rel == rel || e.pkg_rel.starts_with(&rel)
      })
    });
  }

  if entries.is_empty() && root_map.is_none_or(|m| m.is_empty()) {
    return Ok(PackageStatus {
      name: pkg.to_string(),
      status: StatusKind::Unlinked,
      total: 0,
      linked: 0,
      missing: 0,
      conflicts: 0,
      notes: vec!["empty package".to_string()],
      issues: Vec::new(),
    });
  }

  let mut covered_prefixes: Vec<String> = Vec::new(); // pkg-rel prefixes folded into this package
  let mut linked = 0usize;
  let mut missing = 0usize;
  let mut conflicts = 0usize;
  let mut issues = Vec::new();

  for entry in &entries {
    let rel = entry.pkg_rel.to_string_lossy().replace('\\', "/");
    // covered by a folded ancestor owned by this package?
    if covered_prefixes.iter().any(|p| rel.starts_with(p.as_str())) {
      linked += 1;
      continue;
    }
    let dst = target.join(&entry.pkg_rel);
    if entry.kind == EntryKind::Dir {
      if dst.is_symlink() {
        let expected = lexical_normalize(&pkg_root.join(&entry.pkg_rel));
        match resolve_link(&dst) {
          Some(rl) if rl == expected => {
            if fs::metadata(&dst).is_ok() {
              // folded link — covers the whole subtree
              covered_prefixes.push(format!("{rel}/"));
              linked += 1;
            } else {
              // dangling fold — the subtree is NOT reachable through it
              conflicts += 1;
              issues.push(EntryIssue {
                target: dst.display().to_string(),
                reason: "dangling fold link".to_string(),
              });
            }
          }
          Some(rl) => {
            // fold resolves to a different subtree — status and the planner disagree
            conflicts += 1;
            issues.push(EntryIssue {
              target: dst.display().to_string(),
              reason: format!("fold points at {}", rl.display()),
            });
          }
          None => {
            conflicts += 1;
            issues.push(EntryIssue {
              target: dst.display().to_string(),
              reason: "unreadable fold link".to_string(),
            });
          }
        }
      } else if fs::symlink_metadata(&dst).is_ok() && !dst.is_dir() {
        // A real file sits where the package expects a directory. The
        // planner rejects this as ConflictReason::DirFileMismatch
        // (link.rs `plan_dir`, Probe::RealFile arm); status must agree.
        //
        // NOTE: the planner's `Probe`/`dst_probe` and conflict verdicts are
        // private to link.rs and out of scope here, so this classification is
        // intentionally duplicated. Keep it in sync with the planner's
        // Probe::RealFile -> DirFileMismatch arm.
        conflicts += 1;
        issues.push(EntryIssue {
          target: dst.display().to_string(),
          reason: "file where directory expected".to_string(),
        });
      }
      // real dir (or missing) — children are checked individually
      continue;
    }
    // leaf entry
    match fs::symlink_metadata(&dst) {
      Ok(meta) if meta.file_type().is_symlink() => {
        if is_pkg_link(&dst, &pkg_root) {
          if fs::metadata(&dst).is_ok() {
            linked += 1;
          } else {
            missing += 1;
            issues.push(EntryIssue {
              target: dst.display().to_string(),
              reason: "dangling link".to_string(),
            });
          }
        } else if let Some(rl) = resolve_link(&dst) {
          if rl.starts_with(source) {
            conflicts += 1;
            issues.push(EntryIssue {
              target: dst.display().to_string(),
              reason: format!("points into another package: {}", rl.display()),
            });
          } else {
            conflicts += 1;
            issues.push(EntryIssue {
              target: dst.display().to_string(),
              reason: format!("points elsewhere: {}", rl.display()),
            });
          }
        } else {
          conflicts += 1;
          issues.push(EntryIssue {
            target: dst.display().to_string(),
            reason: "unreadable link".to_string(),
          });
        }
      }
      Ok(meta) if meta.is_dir() => {
        conflicts += 1;
        issues.push(EntryIssue {
          target: dst.display().to_string(),
          reason: "is a real directory".to_string(),
        });
      }
      Ok(_) => {
        conflicts += 1;
        issues.push(EntryIssue {
          target: dst.display().to_string(),
          reason: "target exists".to_string(),
        });
      }
      Err(_) => {
        missing += 1;
        issues.push(EntryIssue {
          target: dst.display().to_string(),
          reason: "not linked".to_string(),
        });
      }
    }
  }

  // Root-mapped links live outside the target tree (ADR 0001). Count them in
  // the package's row and explain any problem at the absolute destination.
  let mut root_total = 0usize;
  let mut root_linked = 0usize;
  let mut root_missing = 0usize;
  let mut root_conflicts = 0usize;
  if let Some(mappings) = root_map {
    for (key, value) in mappings {
      root_total += 1;
      let (abs_src, dst) = match link::root_destination(pkg, &pkg_root, key, value) {
        Ok((abs_src, dst, _is_dir)) => (abs_src, dst),
        Err(err) => {
          root_conflicts += 1;
          issues.push(EntryIssue {
            target: value.clone(),
            reason: err.to_string(),
          });
          continue;
        }
      };
      match fs::symlink_metadata(&dst) {
        Ok(meta) if meta.file_type().is_symlink() => match resolve_link(&dst) {
          Some(rl) if rl == lexical_normalize(&abs_src) => {
            if fs::metadata(&dst).is_ok() {
              root_linked += 1;
            } else {
              root_missing += 1;
              issues.push(EntryIssue {
                target: dst.display().to_string(),
                reason: "dangling root link".to_string(),
              });
            }
          }
          Some(rl) => {
            root_conflicts += 1;
            issues.push(EntryIssue {
              target: dst.display().to_string(),
              reason: format!("root link points at {}", rl.display()),
            });
          }
          None => {
            root_conflicts += 1;
            issues.push(EntryIssue {
              target: dst.display().to_string(),
              reason: "unreadable root link".to_string(),
            });
          }
        },
        Ok(_) => {
          root_conflicts += 1;
          issues.push(EntryIssue {
            target: dst.display().to_string(),
            reason: "root destination exists".to_string(),
          });
        }
        Err(_) => {
          root_missing += 1;
          issues.push(EntryIssue {
            target: dst.display().to_string(),
            reason: "not linked".to_string(),
          });
        }
      }
    }
  }
  linked += root_linked;
  missing += root_missing;
  conflicts += root_conflicts;

  let total = entries.len() + root_total;
  let status = if conflicts > 0 {
    StatusKind::Conflict
  } else if linked == 0 {
    StatusKind::Unlinked
  } else if missing > 0 {
    StatusKind::Partial
  } else {
    StatusKind::Linked
  };

  let mut notes = Vec::new();
  if root_map.is_some_and(|m| !m.is_empty()) {
    notes.push("root".to_string());
  }
  if missing > 0 {
    notes.push(format!("{missing} not linked"));
  }
  if conflicts > 0 {
    notes.push(format!("{conflicts} conflicts"));
  }

  Ok(PackageStatus {
    name: pkg.to_string(),
    status,
    total,
    linked,
    missing,
    conflicts,
    notes,
    issues,
  })
}

pub fn compute_many(
  source: &Path,
  target: &Path,
  pkgs: &[String],
  ignore: &Ignore,
  roots: &RootMappings,
) -> Result<Vec<PackageStatus>> {
  pkgs
    .iter()
    .map(|p| compute_package(source, target, p, ignore, roots))
    .collect()
}

pub fn summarize(packages: &[PackageStatus]) -> Summary {
  let mut s = Summary {
    total: packages.len(),
    linked: 0,
    partial: 0,
    unlinked: 0,
    conflict: 0,
  };
  for p in packages {
    match p.status {
      StatusKind::Linked => s.linked += 1,
      StatusKind::Partial => s.partial += 1,
      StatusKind::Unlinked => s.unlinked += 1,
      StatusKind::Conflict => s.conflict += 1,
    }
  }
  s
}

pub fn render_short(summary: &Summary) -> String {
  let mut parts = vec![format!("{} package(s)", summary.total)];
  if summary.linked > 0 {
    parts.push(format!("{} linked", summary.linked));
  }
  if summary.partial > 0 {
    parts.push(format!("{} partial", summary.partial));
  }
  if summary.unlinked > 0 {
    parts.push(format!("{} unlinked", summary.unlinked));
  }
  if summary.conflict > 0 {
    parts.push(format!("{} conflicted", summary.conflict));
  }
  parts.join(", ")
}

fn status_colored(kind: StatusKind, no_color: bool) -> String {
  let s = kind.as_str();
  if no_color {
    return s.to_string();
  }
  use owo_colors::OwoColorize;
  match kind {
    StatusKind::Linked => s.green().to_string(),
    StatusKind::Partial => s.yellow().to_string(),
    StatusKind::Unlinked => s.red().to_string(),
    StatusKind::Conflict => s.bright_red().bold().to_string(),
  }
}

pub fn render_table(packages: &[PackageStatus], no_color: bool) -> String {
  use tabled::builder::Builder;
  use tabled::settings::style::Style;

  let mut builder = Builder::default();
  builder.push_record(["Package", "Status", "Notes"]);
  for p in packages {
    let notes = if p.notes.is_empty() {
      "ok".to_string()
    } else {
      p.notes.join(", ")
    };
    builder.push_record([p.name.clone(), status_colored(p.status, no_color), notes]);
  }
  let mut table = builder.build();
  table.with(Style::rounded()).to_string()
}

pub fn render_json(packages: &[PackageStatus]) -> Result<String> {
  let summary = summarize(packages);
  let out = JsonOutput {
    version: 1,
    packages: packages.to_vec(),
    summary,
  };
  serde_json::to_string_pretty(&out).map_err(|e| Error::Message(e.to_string()))
}

/// Exit code: 0 iff every inspected package is fully linked and conflict-free.
pub fn exit_code(packages: &[PackageStatus]) -> i32 {
  if packages.iter().all(|p| p.status == StatusKind::Linked) {
    0
  } else {
    1
  }
}

const VERBOSE_ISSUE_CAP: usize = 20;

fn cap_issues(cap: usize, all: bool) -> usize {
  if all { usize::MAX } else { cap }
}

/// Human-oriented verbose rendering: per package, one header line with counts,
/// then one line per issue (capped), plus a pointer to --json when truncated.
pub fn render_verbose(packages: &[PackageStatus], no_color: bool, all: bool) -> String {
  use owo_colors::OwoColorize;

  let mut out = String::new();
  for p in packages {
    let notes = if p.notes.is_empty() {
      "ok".to_string()
    } else {
      p.notes.join(", ")
    };
    out.push_str(&format!(
      "{}: {} — {}\n",
      p.name,
      status_colored(p.status, no_color),
      notes
    ));
    for issue in p.issues.iter().take(cap_issues(VERBOSE_ISSUE_CAP, all)) {
      let marker = if issue.reason.starts_with("not linked") {
        if no_color {
          "missing".to_string()
        } else {
          "missing".yellow().to_string()
        }
      } else if no_color {
        "conflict".to_string()
      } else {
        "conflict".bright_red().bold().to_string()
      };
      // conflicts carry a useful reason; "not linked" markers are self-explanatory
      let line = if issue.reason.starts_with("not linked") {
        format!("  {marker}  {}\n", issue.target)
      } else {
        format!("  {marker}  {} — {}\n", issue.target, issue.reason)
      };
      out.push_str(&line);
    }
    let remaining = p
      .issues
      .len()
      .saturating_sub(cap_issues(VERBOSE_ISSUE_CAP, all));
    if remaining > 0 {
      out.push_str(&format!(
        "  ... and {remaining} more (--json for the full list)\n"
      ));
    }
  }
  out
}
