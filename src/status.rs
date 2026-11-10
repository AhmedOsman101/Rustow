//! `rustow status` — per-package link health, table/short/JSON rendering.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{Error, Result};
use crate::ignore::Ignore;
use crate::link::{EntryKind, lexical_normalize, scan_package};

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
) -> Result<PackageStatus> {
  let entries = scan_package(source, pkg, ignore)?;
  let pkg_root = source.join(pkg);

  if entries.is_empty() {
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
      if dst.is_symlink() && is_pkg_link(&dst, &pkg_root) && fs::metadata(&dst).is_ok() {
        // folded link — covers the whole subtree
        covered_prefixes.push(format!("{rel}/"));
        linked += 1;
      } else if dst.is_symlink() && is_pkg_link(&dst, &pkg_root) {
        // dangling fold — the subtree is NOT reachable through it; children report individually
        conflicts += 1;
        issues.push(EntryIssue {
          target: dst.display().to_string(),
          reason: "dangling fold link".to_string(),
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

  let total = entries.len();
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
) -> Result<Vec<PackageStatus>> {
  pkgs
    .iter()
    .map(|p| compute_package(source, target, p, ignore))
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
