//! Elevation helper for root-mapped actions (ADR 0001).
//!
//! Rustow never elevates the whole run. Only the handful of actions whose
//! destination is an absolute, root-mapped path go through this helper; every
//! other action stays with the invoking user. The elevation program receives
//! the action as its argv (`<program> ln -s <src> <dst>`), so `sudo`, `doas`
//! and a purpose-built helper script all share one interface.
//!
//! Availability and destination-parent writability are checked *before* any
//! filesystem mutation (plan-time preflight). The y/n authorization prompt
//! (or `--as-root`) is also handled here.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Error, Result};

/// A probed, ready-to-invoke elevation program.
#[derive(Debug, Clone)]
pub struct Elevation {
  program: PathBuf,
  /// Human-readable description used in error messages.
  origin: String,
}

impl Elevation {
  /// Probe elevation availability: configured `elevate_cmd` first, else
  /// `sudo`, else `doas`. A configured command that is missing or not
  /// executable is a clean error (it must not silently fall back to sudo).
  pub fn probe(configured: Option<&str>) -> Result<Self> {
    if let Some(cmd) = configured.filter(|s| !s.trim().is_empty()) {
      let program = resolve_program(cmd).ok_or_else(|| {
        Error::Config(format!(
          "elevation command {cmd:?} (elevate_cmd) is not executable or not on PATH — cannot apply root-mapped actions"
        ))
      })?;
      return Ok(Self {
        program,
        origin: format!("elevate_cmd = {cmd:?}"),
      });
    }
    for name in ["sudo", "doas"] {
      if let Some(program) = resolve_program(name) {
        return Ok(Self {
          program,
          origin: format!("probed {name}"),
        });
      }
    }
    Err(Error::Config(
      "no elevation command available (tried sudo, then doas) — set elevate_cmd in config.toml or re-run with --as-root after configuring it".to_string(),
    ))
  }

  /// Plan-time preflight. For every root destination, the nearest existing
  /// ancestor must be a writable directory; missing ancestors are fine (the
  /// elevation helper creates them with `mkdir -p`). Aborts on the first
  /// problem so the caller can refuse the whole run before touching the fs.
  pub fn preflight(&self, dsts: &HashSet<PathBuf>) -> Result<()> {
    for dst in dsts {
      let mut ancestor = dst.parent().unwrap_or_else(|| Path::new("/"));
      loop {
        match fs::metadata(ancestor) {
          Ok(meta) => {
            if !meta.is_dir() {
              return Err(Error::Config(format!(
                "root destination parent {} is not a directory",
                ancestor.display()
              )));
            }
            if !writable_dir(ancestor) {
              return Err(Error::Config(format!(
                "root destination parent {} is not writable — aborting before any change",
                ancestor.display()
              )));
            }
            break;
          }
          Err(_) => {
            let Some(parent) = ancestor.parent() else {
              return Err(Error::Config(format!(
                "cannot resolve parent of root destination {}",
                dst.display()
              )));
            };
            if parent == ancestor {
              return Err(Error::Config(format!(
                "cannot resolve parent of root destination {}",
                dst.display()
              )));
            }
            ancestor = parent;
          }
        }
      }
    }
    Ok(())
  }

  /// Create one root symlink. Parent directories are created first. An
  /// already-correct link is left alone; any other occupant is an error (the
  /// planner/overwrite machinery owns removal, so elevation never silently
  /// deletes a target that appeared after planning).
  pub fn apply_link(&self, src: &Path, dst: &Path) -> Result<()> {
    if let Ok(raw) = fs::read_link(dst) {
      let resolved = if raw.is_absolute() {
        raw
      } else {
        dst.parent().unwrap_or_else(|| Path::new(".")).join(raw)
      };
      if crate::link::lexical_normalize(&resolved) == crate::link::lexical_normalize(src) {
        return Ok(()); // already correct
      }
      return Err(Error::Message(format!(
        "{} appeared while linking (plan was stale); re-run rustow",
        dst.display()
      )));
    }
    let parent = dst.parent().ok_or_else(|| {
      Error::Message(format!(
        "root destination {} has no parent directory",
        dst.display()
      ))
    })?;
    self.run(&["mkdir", "-p", "--", &parent.to_string_lossy()])?;
    self.run(&[
      "ln",
      "-s",
      "--",
      &src.to_string_lossy(),
      &dst.to_string_lossy(),
    ])
  }

  /// Remove exactly one root-mapped link (unlink).
  pub fn remove(&self, dst: &Path) -> Result<()> {
    self.run(&["rm", "-f", "--", &dst.to_string_lossy()])
  }

  fn run(&self, args: &[&str]) -> Result<()> {
    let status = Command::new(&self.program)
      .args(args)
      .status()
      .map_err(|e| {
        Error::Message(format!(
          "failed to run elevation program {} ({}): {e}",
          self.program.display(),
          self.origin
        ))
      })?;
    if !status.success() {
      return Err(Error::Message(format!(
        "elevation program {} ({}) failed: {} {}",
        self.program.display(),
        self.origin,
        args.join(" "),
        status
      )));
    }
    Ok(())
  }
}

/// Resolve a command name (PATH search) or path to an executable file.
fn resolve_program(name: &str) -> Option<PathBuf> {
  let candidate = Path::new(name);
  if name.contains('/') {
    return executable(candidate).then(|| candidate.to_path_buf());
  }
  let path = std::env::var_os("PATH")?;
  for dir in std::env::split_paths(&path) {
    let candidate = dir.join(name);
    if executable(&candidate) {
      return Some(candidate);
    }
  }
  None
}

#[cfg(unix)]
fn executable(path: &Path) -> bool {
  use std::os::unix::fs::PermissionsExt;
  fs::metadata(path)
    .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    .unwrap_or(false)
}

#[cfg(not(unix))]
fn executable(path: &Path) -> bool {
  path.is_file()
}

#[cfg(unix)]
fn writable_dir(path: &Path) -> bool {
  use std::os::unix::fs::PermissionsExt;
  fs::metadata(path)
    .map(|m| m.permissions().mode() & 0o222 != 0)
    .unwrap_or(false)
}

#[cfg(not(unix))]
fn writable_dir(_path: &Path) -> bool {
  true
}

/// Effective UID is root. Best-effort: Linux exposes it via `/proc`; on other
/// platforms this returns false (the warning is informational, never Load-bearing).
pub fn euid_is_zero() -> bool {
  #[cfg(target_os = "linux")]
  {
    if let Ok(status) = fs::read_to_string("/proc/self/status") {
      for line in status.lines() {
        if let Some(rest) = line.strip_prefix("Uid:") {
          let fields: Vec<_> = rest.split_whitespace().collect();
          return fields.get(1) == Some(&"0"); // index 1 = effective UID
        }
      }
    }
  }
  false
}

/// Authorize elevated actions. `--as-root` authorizes non-interactively;
/// otherwise print the action list and read one y/n line from stdin. EOF (a
/// closed or non-TTY stdin) fails loudly with the `--as-root` hint. `n` (or
/// anything other than y/yes) aborts the whole run.
pub fn authorize(descriptions: &[String], as_root: bool) -> Result<()> {
  if euid_is_zero() {
    eprintln!(
      "rustow: WARNING: running as root — root's config and HOME apply; root-mapped actions follow the same authorization rules"
    );
  }
  if as_root {
    return Ok(());
  }
  eprintln!(
    "rustow: about to apply {} elevated action(s):",
    descriptions.len()
  );
  for line in descriptions {
    eprintln!("  {line}");
  }
  eprint!("proceed with elevated steps? [y/n] ");
  use std::io::Write;
  let _ = std::io::stderr().flush();

  let mut answer = String::new();
  let read = std::io::stdin().read_line(&mut answer).unwrap_or(0);
  if read == 0 {
    return Err(Error::Config(
      "no input available at the elevation prompt — re-run with --as-root to authorize elevated steps non-interactively".to_string(),
    ));
  }
  match answer.trim().to_ascii_lowercase().as_str() {
    "y" | "yes" => Ok(()),
    _ => Err(Error::Aborted),
  }
}
