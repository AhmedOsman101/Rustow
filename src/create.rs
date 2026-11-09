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
}

pub struct CreateContext {
  pub source_dir: PathBuf,
  pub target: PathBuf,
  pub dry_run: bool,
  pub fold: bool,
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

pub fn plan_create(ctx: &mut CreateContext, opts: &CreateOptions) -> Result<CreateOutcome> {
  let app = &opts.app;
  if app.contains('/') || app.contains('\\') || app == "." || app == ".." || app.is_empty() {
    return Err(Error::Message(format!("invalid package name: {app:?}")));
  }
  let default_relpath = format!(".config/{app}");
  let raw_relpath = opts.path.as_deref().unwrap_or(default_relpath.as_str());
  // validate relpath stays inside the package (no .., no absolute)
  if Path::new(raw_relpath).is_absolute() || raw_relpath.split('/').any(|c| c == "..") {
    return Err(Error::Message(format!("invalid --path: {raw_relpath:?}")));
  }
  let relpath = raw_relpath.trim_start_matches('/');
  let pkg = pkg_dir(ctx, app);
  let dst = dst_for(ctx, app, relpath);
  let src = src_for(ctx, app, relpath, opts.source.as_deref());

  // 1. package already exists → never clobber, offer link
  if pkg.exists() {
    eprintln!("package already exists at {}", pkg.display());
    print_pkg_summary(&pkg, opts.verbose)?;
    if opts.yes {
      // --yes accepts linking the existing package; moving is not implied
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

  // both flags → error
  if opts.empty && opts.source.is_some() {
    return Err(Error::Message(
      "--empty and --source are mutually exclusive".to_string(),
    ));
  }

  // 2. --empty → create bare package, stop
  if opts.empty {
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

  // 3. source exists → move it in
  if src.exists() {
    if dst.exists() {
      return Err(Error::Message(format!(
        "destination {} already exists — refusing to overwrite",
        dst.display()
      )));
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
    move_path(&src, &dst)?;
    eprintln!("moved {} -> {}", src.display(), dst.display());
    // default: link right away; -i asks first
    return maybe_link(ctx, opts, app, Some((src, dst)));
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
      .items(&items)
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
