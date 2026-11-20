//! `rustow create <app>` — create a package, optionally adopting an existing
//! target path into it, then link it. Replaces the old mkconf script.
//! `rustow add` is a command alias.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};
use crate::link::{self, ConflictResolver};

#[derive(Debug, Clone)]
pub struct CreateOptions {
  pub app: String,
  /// Package/target-relative destination path (default: .config/<app>).
  pub path: Option<String>,
  pub verbose: bool,
  pub interactive: bool,
  pub empty: bool,
  pub source: Option<PathBuf>,
  pub yes: bool,
  /// -f/--force from the global flags: overwrite colliding package files.
  pub force: bool,
}

pub struct CreateContext {
  pub source_dir: PathBuf,
  pub target: PathBuf,
  pub dry_run: bool,
  pub fold: bool,
  pub fold_ignore: Vec<String>,
  pub ignore_globs: Vec<String>,
  /// Same conflict policy as `link` (selected from global flags/config).
  pub conflict_resolver: Box<dyn ConflictResolver>,
}

#[derive(Debug)]
pub struct CreateOutcome {
  pub created_package: bool,
  pub moved: Option<(PathBuf, PathBuf)>,
  pub linked: bool,
}

fn pkg_dir(ctx: &CreateContext, app: &str) -> PathBuf {
  ctx.source_dir.join(app)
}

fn dst_for(ctx: &CreateContext, app: &str, relpath: &str) -> PathBuf {
  pkg_dir(ctx, app).join(relpath)
}

fn src_for(ctx: &CreateContext, _app: &str, relpath: &str, source: Option<&Path>) -> PathBuf {
  match source {
    Some(s) => s.to_path_buf(),
    None => ctx.target.join(relpath),
  }
}

/// Files under `path` (recursively), as (relpath, size) pairs.
fn list_files(path: &Path) -> Result<Vec<(PathBuf, u64)>> {
  let mut out = Vec::new();
  for entry in walkdir::WalkDir::new(path).follow_links(false) {
    let entry = entry.map_err(|e| Error::Message(e.to_string()))?;
    if entry.file_type().is_file() {
      let rel = entry
        .path()
        .strip_prefix(path)
        .unwrap_or(entry.path())
        .to_path_buf();
      let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
      out.push((rel, size));
    }
  }
  Ok(out)
}

fn total_bytes(files: &[(PathBuf, u64)]) -> u64 {
  files.iter().map(|(_, s)| s).sum()
}

fn fmt_bytes(bytes: u64) -> String {
  const KB: u64 = 1024;
  const MB: u64 = KB * 1024;
  if bytes >= MB {
    format!("{:.1} MB", bytes as f64 / MB as f64)
  } else if bytes >= KB {
    format!("{:.1} KB", bytes as f64 / KB as f64)
  } else {
    format!("{bytes} B")
  }
}

fn confirm_or_yes(yes: bool, prompt: &str) -> Result<bool> {
  if yes {
    eprintln!("{prompt} [--yes]");
    return Ok(true);
  }
  use std::io::IsTerminal;
  if !std::io::stdin().is_terminal() {
    return Err(Error::Message(format!(
      "{prompt}\nrefusing to proceed without --yes (non-interactive)"
    )));
  }
  let confirmed = dialoguer::Confirm::new()
    .with_prompt(prompt)
    .default(false)
    .interact()
    .map_err(|e| Error::Message(format!("prompt failed: {e}")))?;
  Ok(confirmed)
}

/// Move `src` to `dst`, creating parent dirs. Falls back to copy+remove across filesystems.
fn move_path(src: &Path, dst: &Path) -> Result<()> {
  if let Some(parent) = dst.parent() {
    fs::create_dir_all(parent).map_err(Error::io(parent))?;
  }
  if dst.exists() || dst.is_symlink() {
    return Err(Error::Message(format!(
      "refusing to overwrite existing {}",
      dst.display()
    )));
  }
  match fs::rename(src, dst) {
    Ok(()) => Ok(()),
    Err(_) => {
      // cross-device: copy then remove
      if src.is_dir() {
        copy_dir_recursive(src, dst)?;
        fs::remove_dir_all(src).map_err(Error::io(src))?;
      } else {
        fs::copy(src, dst).map_err(Error::io(dst))?;
        fs::remove_file(src).map_err(Error::io(src))?;
      }
      Ok(())
    }
  }
}

/// One file's move from source tree into the package, with collision info.
struct MoveItem {
  from: PathBuf,
  to: PathBuf,
  is_dir: bool,
}

/// Collect the full move plan for merging `src` (file or dir) into `dst` which may exist (dir only).
/// Returns the plan and a list of collisions (dst occupied by something incompatible).
fn collect_move(src: &Path, dst: &Path) -> Result<(Vec<MoveItem>, Vec<PathBuf>)> {
  let mut items = Vec::new();
  let mut collisions = Vec::new();

  if !src.is_dir() {
    // src is a file or symlink — single move (or overwrite under -f)
    if dst.exists() || dst.is_symlink() {
      collisions.push(dst.to_path_buf());
      items.push(MoveItem {
        from: src.to_path_buf(),
        to: dst.to_path_buf(),
        is_dir: false,
      });
    } else {
      items.push(MoveItem {
        from: src.to_path_buf(),
        to: dst.to_path_buf(),
        is_dir: false,
      });
    }
    return Ok((items, collisions));
  }

  // src is a directory
  for entry in walkdir::WalkDir::new(src).follow_links(false) {
    let entry = entry.map_err(|e| Error::Message(e.to_string()))?;
    if entry.path() == src {
      // top-level dir itself — never merge through a symlink at dst
      if dst.exists() || dst.is_symlink() {
        if dst.is_dir() && !dst.is_symlink() {
          continue; // merge into it
        }
        collisions.push(dst.to_path_buf());
      }
      continue;
    }
    let Ok(rel) = entry.path().strip_prefix(src).map(PathBuf::from) else {
      continue;
    };
    let to = dst.join(rel);
    if entry.file_type().is_dir() {
      // dir collision only when dst path is a non-directory
      if to.is_file() || to.is_symlink() {
        collisions.push(to.clone());
      }
      items.push(MoveItem {
        from: entry.path().to_path_buf(),
        to,
        is_dir: true,
      });
    } else {
      // file collision (package file vs incoming file): always include in the plan;
      // without -f the caller aborts before anything is moved
      if to.exists() || to.is_symlink() {
        collisions.push(to.clone());
      }
      items.push(MoveItem {
        from: entry.path().to_path_buf(),
        to,
        is_dir: false,
      });
    }
  }
  Ok((items, collisions))
}

/// Execute a move plan (src tree into possibly-existing dst dir), file by file.
/// `force` allows overwriting colliding package files.
fn execute_move(items: &[MoveItem], force: bool, src_root: &Path) -> Result<()> {
  // create package dirs first (deepest ordering comes free from sorted walk)
  let mut items_sorted: Vec<&MoveItem> = items.iter().collect();
  items_sorted.sort_by_key(|i| i.from.components().count());

  for item in &items_sorted {
    if item.is_dir {
      fs::create_dir_all(&item.to).map_err(Error::io(&item.to))?;
      continue;
    }
    if item.to.exists() || item.to.is_symlink() {
      if !force {
        return Err(Error::Message(format!(
          "refusing to overwrite {}",
          item.to.display()
        )));
      }
      if item.to.is_dir() && !item.to.is_symlink() {
        fs::remove_dir_all(&item.to).map_err(Error::io(&item.to))?;
      } else {
        fs::remove_file(&item.to).map_err(Error::io(&item.to))?;
      }
    }
    if let Some(parent) = item.to.parent() {
      fs::create_dir_all(parent).map_err(Error::io(parent))?;
    }
    match fs::rename(&item.from, &item.to) {
      Ok(()) => {}
      Err(_) => {
        fs::copy(&item.from, &item.to).map_err(Error::io(&item.to))?;
        fs::remove_file(&item.from).map_err(Error::io(&item.from))?;
      }
    }
  }
  // clean up any empty source tree left behind
  let _ = fs::remove_dir_all(src_root);
  Ok(())
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
  fs::create_dir_all(dst).map_err(Error::io(dst))?;
  for entry in fs::read_dir(src).map_err(Error::io(src))?.flatten() {
    let from = entry.path();
    let to = dst.join(entry.file_name());
    if from.is_dir() {
      copy_dir_recursive(&from, &to)?;
    } else {
      fs::copy(&from, &to).map_err(Error::io(&to))?;
    }
  }
  Ok(())
}

/// If `candidate` resolves (lexically or, when it exists, through symlinks) to
/// a path inside `source_dir`, return the owning package name. Under a fold,
/// `target/.config/<app>` is a symlink into another package's tree — adopting it
/// would silently relocate that package's files.
fn package_owning_path(source_dir: &Path, candidate: &Path) -> Option<String> {
  let source_lex = crate::link::lexical_normalize(source_dir);
  let source_canon = fs::canonicalize(source_dir).unwrap_or_else(|_| source_lex.clone());
  let candidate_lex = crate::link::lexical_normalize(candidate);
  let candidate_canon = fs::canonicalize(candidate).unwrap_or_else(|_| candidate_lex.clone());

  for root in [&source_lex, &source_canon] {
    for resolved in [&candidate_lex, &candidate_canon] {
      if let Ok(rel) = resolved.strip_prefix(root)
        && let Some(std::path::Component::Normal(name)) = rel.components().next()
      {
        return Some(name.to_string_lossy().into_owned());
      }
    }
  }
  None
}

/// Single guard for every `create` adoption route: the implicit target path,
/// `--source`, and the interactive TTY prompter. Refuse a candidate that is the
/// package itself (or an ancestor of it), and any candidate that really lives
/// inside the source tree — those files belong to a package and moving them
/// would relocate it. Both checks resolve lexically and through symlinks.
fn guard_adoption(source_dir: &Path, pkg: &Path, candidate: &Path, label: &str) -> Result<()> {
  let pkg_lex = crate::link::lexical_normalize(pkg);
  let candidate_lex = crate::link::lexical_normalize(candidate);
  let pkg_canon = fs::canonicalize(pkg).unwrap_or_else(|_| pkg_lex.clone());
  let candidate_canon = fs::canonicalize(candidate).unwrap_or_else(|_| candidate_lex.clone());
  let overlaps = |a: &Path, b: &Path| a == b || a.starts_with(b) || b.starts_with(a);
  if overlaps(&candidate_lex, &pkg_lex) || overlaps(&candidate_canon, &pkg_canon) {
    return Err(Error::Message(format!(
      "{label} {} must not be the package itself or an ancestor of it",
      candidate.display()
    )));
  }

  if let Some(owner) = package_owning_path(source_dir, candidate) {
    return Err(Error::Message(format!(
      "source {} is inside the source tree and belongs to package {owner:?}; refusing to adopt a path already managed by a package",
      candidate.display()
    )));
  }
  Ok(())
}

pub fn plan_create(ctx: &mut CreateContext, opts: &CreateOptions) -> Result<CreateOutcome> {
  let app = &opts.app;
  if app.contains('/') || app.contains('\\') || app == "." || app == ".." || app.is_empty() {
    return Err(Error::Message(format!("invalid package name: {app:?}")));
  }
  let default_relpath = format!(".config/{app}");
  let raw_relpath = opts.path.as_deref().unwrap_or(default_relpath.as_str());
  // validate relpath stays inside the package: no absolute paths, no `..`,
  // and it must contain at least one real (Normal) component — otherwise
  // `--path .` would rename the whole target root into the package
  if Path::new(raw_relpath).is_absolute() || raw_relpath.split('/').any(|c| c == "..") {
    return Err(Error::Message(format!("invalid --path: {raw_relpath:?}")));
  }
  let relpath = raw_relpath.trim_start_matches('/');
  if !Path::new(relpath)
    .components()
    .any(|c| matches!(c, std::path::Component::Normal(_)))
  {
    return Err(Error::Message(format!(
      "invalid --path: {raw_relpath:?} (needs at least one path component)"
    )));
  }
  let pkg = pkg_dir(ctx, app);
  let dst = dst_for(ctx, app, relpath);
  let src = src_for(ctx, app, relpath, opts.source.as_deref());
  // Every adoption route goes through the same guard, before any fs mutation.
  if let Some(explicit) = opts.source.as_ref() {
    guard_adoption(&ctx.source_dir, &pkg, explicit, "--source")?;
  } else if !opts.empty {
    guard_adoption(&ctx.source_dir, &pkg, &src, "source")?;
  }

  // both flags → error
  if opts.empty && opts.source.is_some() {
    return Err(Error::Message(
      "--empty and --source are mutually exclusive".to_string(),
    ));
  }

  // --empty → create bare package, stop
  if opts.empty {
    if pkg.exists() {
      eprintln!("package already exists at {}", pkg.display());
      return Err(Error::Message(format!(
        "package {} already exists",
        pkg.display()
      )));
    }
    if ctx.dry_run {
      println!("would create empty package {}", pkg.display());
    } else {
      fs::create_dir_all(&pkg).map_err(Error::io(&pkg))?;
      println!("created empty package {}", pkg.display());
      println!("populate it, then run `rustow link {app}`");
    }
    return Ok(CreateOutcome {
      created_package: true,
      moved: None,
      linked: false,
    });
  }

  // source exists (pkg may or may not exist) → move/merge into the package
  if src.exists() {
    // a symlink at the package destination would redirect the whole move
    // outside the package — never move through it
    if dst.is_symlink() {
      if !opts.force {
        return Err(Error::Conflicts(format!(
          "{} is a symlink — refusing to move the source through it (use -f/--force to remove it first)",
          dst.display()
        )));
      }
      eprintln!("warning: removing symlink at {} (-f)", dst.display());
      fs::remove_file(&dst).map_err(Error::io(&dst))?;
    }
    let (items, collisions) = collect_move(&src, &dst)?;
    if !collisions.is_empty() {
      if !opts.force {
        let report = collisions
          .iter()
          .map(|c| format!("  {}", c.display()))
          .collect::<Vec<_>>()
          .join("\n");
        return Err(Error::Conflicts(format!(
          "rustow: destination files already exist in the package — nothing was moved:\n{report}\nuse -f/--force to overwrite them"
        )));
      }
      eprintln!(
        "warning: overwriting {} existing file(s) in the package (-f)",
        collisions.len()
      );
    }
    if items.is_empty() && collisions.is_empty() {
      eprintln!("nothing to move from {}", src.display());
      return maybe_link(ctx, opts, app, None);
    }
    let files = list_files(&src)?;
    let total = total_bytes(&files);
    eprintln!(
      "source {}\ndestination {}\n{} files, {}",
      src.display(),
      dst.display(),
      files.len(),
      fmt_bytes(total)
    );
    if opts.verbose {
      for (rel, size) in &files {
        eprintln!("  {} ({})", rel.display(), fmt_bytes(*size));
      }
    }
    let accepted = confirm_or_yes(
      opts.yes,
      &format!("move {} into {}?", src.display(), pkg.display()),
    )?;
    if !accepted {
      return Err(Error::Aborted);
    }
    if ctx.dry_run {
      println!("would move {} to {}", src.display(), dst.display());
      return Ok(CreateOutcome {
        created_package: true,
        moved: Some((src, dst)),
        linked: false,
      });
    }
    if !dst.exists() {
      // fast path: single rename
      move_path(&src, &dst)?;
    } else {
      // merge: src dir into existing dst dir
      execute_move(&items, opts.force, &src)?;
    }
    eprintln!("moved {} -> {}", src.display(), dst.display());
    // default: link right away; -i asks first
    return maybe_link(ctx, opts, app, Some((src, dst)));
  }

  // package exists, no source → link it (or abort)
  if pkg.exists() {
    eprintln!("package already exists at {}", pkg.display());
    print_pkg_summary(&pkg, opts.verbose)?;
    if opts.yes {
      return link_existing(ctx, app);
    }
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
      return Err(Error::Message(format!(
        "package {} already exists — use `rustow link {app}` to link it, or --yes to confirm linking",
        pkg.display()
      )));
    }
    let link_it = dialoguer::Confirm::new()
      .with_prompt("link the existing package?")
      .default(true)
      .interact()
      .map_err(|e| Error::Message(format!("prompt failed: {e}")))?;
    if link_it {
      return link_existing(ctx, app);
    }
    return Ok(CreateOutcome {
      created_package: false,
      moved: None,
      linked: false,
    });
  }

  // 4. source missing
  if let Some(explicit) = opts.source.as_ref() {
    return Err(Error::Message(format!(
      "--source {} does not exist",
      explicit.display()
    )));
  }
  use std::io::IsTerminal;
  if std::io::stdin().is_terminal() {
    let items = [
      "create an empty package (populate it yourself)",
      "take source from another path",
      "abort",
    ];
    let choice = dialoguer::Select::new()
      .with_prompt(format!("no source found at {}", src.display()))
      .items(items)
      .default(0)
      .interact()
      .map_err(|e| Error::Message(format!("prompt failed: {e}")))?;
    match choice {
      0 => {
        if ctx.dry_run {
          println!("would create empty package {}", pkg.display());
        } else {
          fs::create_dir_all(&pkg).map_err(Error::io(&pkg))?;
          println!("created empty package {}", pkg.display());
        }
        return Ok(CreateOutcome {
          created_package: true,
          moved: None,
          linked: false,
        });
      }
      1 => {
        let path: String = dialoguer::Input::new()
          .with_prompt("path to source (file or directory)")
          .interact_text()
          .map_err(|e| Error::Message(format!("prompt failed: {e}")))?;
        let explicit = PathBuf::from(path);
        if !explicit.exists() {
          return Err(Error::Message(format!(
            "{} does not exist",
            explicit.display()
          )));
        }
        guard_adoption(&ctx.source_dir, &pkg, &explicit, "source")?;
        let files = list_files(&explicit)?;
        eprintln!(
          "source {}\ndestination {}\n{} files, {}",
          explicit.display(),
          dst.display(),
          files.len(),
          fmt_bytes(total_bytes(&files))
        );
        if !confirm_or_yes(opts.yes, "move it into the package?")? {
          return Err(Error::Aborted);
        }
        if !ctx.dry_run {
          move_path(&explicit, &dst)?;
          eprintln!("moved {} -> {}", explicit.display(), dst.display());
        } else {
          println!("would move {} to {}", explicit.display(), dst.display());
        }
        return maybe_link(ctx, opts, app, Some((explicit, dst)));
      }
      _ => return Err(Error::Aborted),
    }
  }
  Err(Error::Message(format!(
    "no source found at {} — use --empty to create an empty package, or --source <path>",
    src.display()
  )))
}

fn link_existing(ctx: &mut CreateContext, app: &str) -> Result<CreateOutcome> {
  if ctx.dry_run {
    println!("would link {app}");
    return Ok(CreateOutcome {
      created_package: false,
      moved: None,
      linked: false,
    });
  }
  let ignore = crate::ignore::Ignore::new(&ctx.ignore_globs)?;
  let pkgs = vec![app.to_string()];
  let plans = link::plan_link_all(&link::LinkInput {
    source: &ctx.source_dir,
    target: &ctx.target,
    pkgs: &pkgs,
    fold: ctx.fold,
    fold_ignore: &ctx.fold_ignore,
    ignore: &ignore,
  })?;
  // conflicts abort with the full report (prebuilt by apply_plans); nothing is changed
  link::apply_plans(
    &plans,
    &ctx.source_dir,
    ctx.conflict_resolver.as_mut(),
    false,
  )?;
  println!("linked {app}");
  Ok(CreateOutcome {
    created_package: false,
    moved: None,
    linked: true,
  })
}

fn maybe_link(
  ctx: &mut CreateContext,
  opts: &CreateOptions,
  app: &str,
  moved: Option<(PathBuf, PathBuf)>,
) -> Result<CreateOutcome> {
  // default: link; -i asks first
  let link = if opts.interactive {
    use std::io::IsTerminal;
    if !std::io::stdin().is_terminal() {
      true // non-interactive: link by default
    } else {
      dialoguer::Confirm::new()
        .with_prompt("link the package now?")
        .default(true)
        .interact()
        .map_err(|e| Error::Message(format!("prompt failed: {e}")))?
    }
  } else {
    true
  };
  if link {
    let mut outcome = link_existing(ctx, app)?;
    outcome.moved = moved;
    outcome.created_package = true;
    return Ok(outcome);
  }
  Ok(CreateOutcome {
    created_package: true,
    moved,
    linked: false,
  })
}

fn print_pkg_summary(pkg: &Path, verbose: bool) -> Result<()> {
  let files = list_files(pkg)?;
  eprintln!("{} files, {}", files.len(), fmt_bytes(total_bytes(&files)));
  if verbose {
    for (rel, size) in &files {
      eprintln!("  {} ({})", rel.display(), fmt_bytes(*size));
    }
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  // The interactive TTY branch is not black-box testable without a PTY; these
  // tests pin the shared guard it calls, which is the same guard used by the
  // implicit-path and --source routes (covered end-to-end in tests/create.rs).

  #[test]
  fn guard_adoption_refuses_path_inside_source_tree_and_names_owner() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("Configs");
    let owned = source.join("base/.config/nvim");
    fs::create_dir_all(&owned).unwrap();
    let pkg = source.join("nvim");

    let err = guard_adoption(&source, &pkg, &owned, "source").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("base"), "{msg}");
    assert!(msg.contains("inside the source tree"), "{msg}");
  }

  #[cfg(unix)]
  #[test]
  fn guard_adoption_refuses_symlink_into_source_tree_and_names_owner() {
    use std::os::unix::fs::symlink;
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("Configs");
    let owned = source.join("base/.config/nvim");
    fs::create_dir_all(&owned).unwrap();
    let home = tmp.path().join("home/.config");
    fs::create_dir_all(&home).unwrap();
    symlink(&owned, home.join("nvim")).unwrap();
    let pkg = source.join("nvim");

    let err = guard_adoption(&source, &pkg, &home.join("nvim"), "source").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("base"), "{msg}");
    assert!(msg.contains("inside the source tree"), "{msg}");
  }

  #[test]
  fn guard_adoption_allows_unrelated_target_path() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("Configs");
    let pkg = source.join("nvim");
    let unrelated = tmp.path().join("home/.config/nvim");
    fs::create_dir_all(&unrelated).unwrap();

    guard_adoption(&source, &pkg, &unrelated, "source").unwrap();
  }

  #[test]
  fn guard_adoption_refuses_package_itself_with_caller_label() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("Configs");
    let pkg = source.join("nvim");
    fs::create_dir_all(&pkg).unwrap();

    let err = guard_adoption(&source, &pkg, &pkg, "--source").unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("--source"), "{msg}");
    assert!(msg.contains("must not be the package itself"), "{msg}");
  }
}
