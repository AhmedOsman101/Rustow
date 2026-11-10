//! clap definitions and dispatch.

use std::fs;
use std::path::{Path, PathBuf};

use clap::{Args, Parser, Subcommand};

use crate::config::{self, CliOverrides};
use crate::error::{Error, Result};
use crate::ignore::Ignore;
use crate::link::{
  self, AbortResolver, ConflictResolution, ConflictResolver, FixedResolver, LinkInput,
  PromptOutcome,
};

#[derive(Debug, Parser)]
#[command(
  name = "rustow",
  version,
  about = "A Stow-compatible dotfiles linker written in Rust",
  long_about = "rustow links a package tree from a stow-style source directory onto a target root (usually $HOME) as symlinks."
)]
pub struct Cli {
  #[command(flatten)]
  pub global: GlobalArgs,

  #[command(subcommand)]
  pub command: Commands,
}

#[derive(Debug, Clone, Args)]
pub struct GlobalArgs {
  /// Dry run — show what would happen without changing anything.
  #[arg(short = 'n', long, global = true)]
  pub dry_run: bool,

  /// Overwrite conflicting targets without asking.
  #[arg(short = 'f', long, global = true, conflicts_with = "adopt")]
  pub force: bool,

  /// Move conflicting target files into the package, then link (stow --adopt).
  #[arg(long, global = true)]
  pub adopt: bool,

  /// Verbose output (-v per-package detail, -vv uncapped).
  #[arg(short, long, global = true, action = clap::ArgAction::Count)]
  pub verbose: u8,

  /// Stow directory (source of packages); overrides config/env/stowrc.
  #[arg(long, global = true, value_name = "DIR")]
  pub dir: Option<PathBuf>,

  /// Link destination root; overrides config/env/stowrc.
  #[arg(long, global = true, value_name = "DIR")]
  pub target: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum Commands {
  /// Inspect or write the rustow configuration.
  #[command(subcommand)]
  Config(ConfigCommands),

  /// Link all packages in the source directory.
  Sync,

  /// Link specific packages.
  Link {
    /// Packages to link.
    #[arg(required = true, value_name = "PACKAGE")]
    pkgs: Vec<String>,
  },

  /// Unlink specific packages (aliases: remove, rm).
  #[command(alias = "remove", alias = "rm")]
  Unlink {
    /// Packages to unlink.
    #[arg(required = true, value_name = "PACKAGE")]
    pkgs: Vec<String>,
  },

  /// Unlink all packages in the source directory.
  Reset,

  /// Create a new package (alias: add).
  #[command(alias = "add")]
  Create {
    /// Package name, e.g. "vim".
    #[arg(value_name = "APP")]
    app: String,
    /// Package/target-relative destination path (default: .config/<app>).
    #[arg(long, value_name = "PATH")]
    path: Option<String>,
    /// Prompt before linking the package.
    #[arg(short, long)]
    interactive: bool,
    /// Create an empty package only; populate it yourself.
    #[arg(long)]
    empty: bool,
    /// Take the source from a custom path instead of $TARGET/<path>.
    #[arg(long, value_name = "SRC")]
    source: Option<PathBuf>,
    /// Skip confirmations.
    #[arg(long)]
    yes: bool,
  },

  /// Unlink then re-link packages (restow).
  Relink {
    /// Packages to relink.
    #[arg(required = true, value_name = "PACKAGE")]
    pkgs: Vec<String>,
  },

  /// Show link health of packages.
  Status {
    /// Packages to inspect (default: all packages).
    #[arg(value_name = "PACKAGE")]
    pkgs: Vec<String>,
    /// Print only the summary line.
    #[arg(short, long, conflicts_with = "verbose")]
    short: bool,
    /// Print machine-readable JSON.
    #[arg(long)]
    json: bool,
    /// Only show packages in these states (repeatable): linked, partial, unlinked, conflict.
    #[arg(long = "state", value_name = "STATE", value_enum)]
    states: Vec<StatusFilter>,
  },
}

/// Status filter for `status --state`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum StatusFilter {
  Linked,
  Partial,
  Unlinked,
  Conflict,
}

impl StatusFilter {
  fn matches(self, kind: crate::status::StatusKind) -> bool {
    use crate::status::StatusKind as K;
    matches!(
      (self, kind),
      (StatusFilter::Linked, K::Linked)
        | (StatusFilter::Partial, K::Partial)
        | (StatusFilter::Unlinked, K::Unlinked)
        | (StatusFilter::Conflict, K::Conflict)
    )
  }
}

#[derive(Debug, Subcommand)]
pub enum ConfigCommands {
  /// Print the annotated default configuration to stdout.
  Default,
  /// Write the default configuration to ~/.config/rustow/config.toml.
  Init {
    /// Overwrite an existing config file.
    #[arg(long)]
    force: bool,
    /// Write to a custom path instead of the default location.
    #[arg(long, value_name = "PATH")]
    path: Option<PathBuf>,
  },
}

/// Direct subdirectories of the source dir, sorted by name.
pub fn list_packages(source: &Path) -> Result<Vec<String>> {
  let mut out = Vec::new();
  let rd = fs::read_dir(source).map_err(Error::io(source))?;
  for item in rd.flatten() {
    if item.file_type().map(|t| t.is_dir()).unwrap_or(false)
      && let Some(name) = item.file_name().to_str()
    {
      out.push(name.to_string());
    }
  }
  out.sort();
  Ok(out)
}

struct App<'a> {
  globals: &'a GlobalArgs,
  resolved: config::ResolvedConfig,
}

impl<'a> App<'a> {
  fn new(globals: &'a GlobalArgs) -> Result<Self> {
    let resolved = config::resolve(&CliOverrides {
      dir: globals.dir.clone(),
      target: globals.target.clone(),
    })?;
    config::notice(&resolved)?;
    Ok(Self { globals, resolved })
  }

  fn ignore(&self) -> Result<Ignore> {
    Ignore::new(&self.resolved.cfg.ignore)
  }

  fn source(&self) -> &Path {
    &self.resolved.source
  }

  fn target(&self) -> &Path {
    &self.resolved.target
  }

  fn make_resolver(&self) -> Box<dyn ConflictResolver> {
    use crate::config::ConflictPolicy;
    use std::io::IsTerminal;

    if self.globals.force {
      return Box::new(FixedResolver(ConflictResolution::Overwrite));
    }
    if self.globals.adopt {
      return Box::new(FixedResolver(ConflictResolution::Adopt));
    }
    match self.resolved.cfg.on_conflict {
      ConflictPolicy::Abort => Box::new(AbortResolver),
      ConflictPolicy::Skip => Box::new(FixedResolver(ConflictResolution::Skip)),
      ConflictPolicy::Overwrite => Box::new(FixedResolver(ConflictResolution::Overwrite)),
      ConflictPolicy::Adopt => Box::new(FixedResolver(ConflictResolution::Adopt)),
      ConflictPolicy::Prompt => {
        if std::io::stdin().is_terminal() {
          Box::new(PromptResolver {
            diff_tool: self.resolved.cfg.diff_tool.clone(),
          })
        } else {
          Box::new(AbortResolver)
        }
      }
    }
  }

  fn link_pkgs(&self, pkgs: &[String]) -> Result<i32> {
    let ignore = self.ignore()?;
    let plans = link::plan_link_all(&LinkInput {
      source: self.source(),
      target: self.target(),
      pkgs,
      fold: self.resolved.cfg.fold,
      ignore: &ignore,
    })?;

    let mut resolver = self.make_resolver();
    match link::apply_plans(
      &plans,
      self.source(),
      resolver.as_mut(),
      self.globals.dry_run,
    ) {
      Ok(out) => {
        if self.resolved.cfg.verbose || self.globals.verbose > 0 {
          for plan in &plans {
            for action in &plan.actions {
              println!("{}", render_action(action));
            }
          }
        }
        for warning in &out.warnings {
          eprintln!("warning: {warning}");
        }
        let verb = if out.dry_run { "would link" } else { "linked" };
        println!(
          "{verb} {} path(s){}, skipped {}",
          out.linked + out.repaired,
          if out.repaired > 0 {
            format!(" ({} repaired)", out.repaired)
          } else {
            String::new()
          },
          out.skipped
        );
        Ok(0)
      }
      Err(e) => Err(e),
    }
  }

  fn unlink_pkgs(&self, pkgs: &[String]) -> Result<i32> {
    let ignore = self.ignore()?;
    let plan = link::plan_unlink(self.source(), self.target(), pkgs, &ignore)?;
    let n = link::apply_unlink(
      &plan,
      self.target(),
      self.source(),
      &ignore,
      self.globals.dry_run,
    )?;
    let verb = if self.globals.dry_run {
      "would unlink"
    } else {
      "unlinked"
    };
    if self.resolved.cfg.verbose || self.globals.verbose > 0 {
      for action in &plan.actions {
        let link::UnlinkAction::Unlink { dst } = action;
        println!("unlink {}", dst.display());
      }
    }
    println!(
      "{verb} {n} link(s){}",
      if plan.foreign_skipped.is_empty() {
        String::new()
      } else {
        format!(
          ", {} foreign entr(ies) left alone",
          plan.foreign_skipped.len()
        )
      }
    );
    Ok(0)
  }
}

fn render_action(action: &link::Action) -> String {
  match action {
    link::Action::Mkdir { dst } => format!("mkdir  {}", dst.display()),
    link::Action::Fold { src_dir, dst } => {
      format!("fold  {} -> {}", dst.display(), src_dir.display())
    }
    link::Action::Link { src, dst } => format!("link  {} -> {}", dst.display(), src.display()),
    link::Action::Unfold { dst } => format!("unfold {}", dst.display()),
    link::Action::Repair { src, dst } => format!("repair {} -> {}", dst.display(), src.display()),
    link::Action::SkipIdempotent { .. } => "skip  (already linked)".to_string(),
  }
}

/// Interactive per-conflict prompt (TTY). File conflicts offer merge via the
/// configured/detected diff tool; everything else offers overwrite/skip/abort.
struct PromptResolver {
  diff_tool: Option<String>,
}

impl ConflictResolver for PromptResolver {
  fn resolve(&mut self, conflict: &link::Conflict) -> Result<PromptOutcome> {
    use dialoguer::Select;
    use std::io::IsTerminal;

    if !std::io::stdin().is_terminal() {
      return Ok(PromptOutcome::Abort);
    }
    eprintln!("conflict: {conflict}");
    let is_regular_file = std::fs::symlink_metadata(&conflict.dst)
      .map(|m| m.file_type().is_file())
      .unwrap_or(false);
    let items: Vec<&str> = if is_regular_file {
      vec![
        "merge (open the diff tool; reconcile into the package file)",
        "overwrite (replace it with the package's version)",
        "skip (leave it as it is)",
        "abort everything",
      ]
    } else {
      vec![
        "overwrite (replace it with the package's version)",
        "skip (leave it as it is)",
        "abort everything",
      ]
    };
    // a cancelled prompt (Esc) means "skip this file" per the spec, not a hard error
    let choice = Select::new()
      .with_prompt("what would you like to do?")
      .items(&items)
      .default(0)
      .interact()
      .unwrap_or_else(|e| {
        eprintln!("prompt cancelled: {e} — treating as skip");
        if is_regular_file { 2 } else { 1 } // skip index in both layouts
      });
    if is_regular_file {
      match choice {
        0 => {
          let cfg_tool = self.diff_tool.as_deref();
          let chosen = crate::diff::choose_tool(cfg_tool, true)?;
          let package_file = conflict
            .src
            .clone()
            .ok_or_else(|| Error::Message("merge: conflict has no package source".to_string()))?;
          crate::diff::run_merge(&chosen, &conflict.dst, &package_file)?;
          Ok(PromptOutcome::Merge)
        }
        1 => Ok(PromptOutcome::Overwrite),
        2 => Ok(PromptOutcome::Skip),
        _ => Ok(PromptOutcome::Abort),
      }
    } else {
      Ok(match choice {
        0 => PromptOutcome::Overwrite,
        1 => PromptOutcome::Skip,
        _ => PromptOutcome::Abort,
      })
    }
  }
}

pub fn run(cli: Cli) -> Result<i32> {
  match cli.command {
    Commands::Config(cmd) => match cmd {
      ConfigCommands::Default => {
        print!("{}", config::print_default());
        Ok(0)
      }
      ConfigCommands::Init { force, path } => {
        let path = path.unwrap_or_else(config::config_path);
        let written = config::write_default(&path, force)?;
        println!("wrote {}", written.display());
        Ok(0)
      }
    },
    Commands::Sync => {
      let app = App::new(&cli.global)?;
      let pkgs = list_packages(app.source())?;
      if pkgs.is_empty() {
        println!("no packages found in {}", app.source().display());
        return Ok(0);
      }
      app.link_pkgs(&pkgs)
    }
    Commands::Link { pkgs } => {
      let app = App::new(&cli.global)?;
      app.link_pkgs(&pkgs)
    }
    Commands::Unlink { pkgs } => {
      let app = App::new(&cli.global)?;
      app.unlink_pkgs(&pkgs)
    }
    Commands::Reset => {
      let app = App::new(&cli.global)?;
      let pkgs = list_packages(app.source())?;
      if pkgs.is_empty() {
        println!("no packages found in {}", app.source().display());
        return Ok(0);
      }
      app.unlink_pkgs(&pkgs)
    }
    Commands::Relink { pkgs } => {
      let app = App::new(&cli.global)?;
      // unlink first (never conflicts), then link
      let _ = app.unlink_pkgs(&pkgs)?;
      app.link_pkgs(&pkgs)
    }
    Commands::Create {
      app,
      path,
      interactive,
      empty,
      source,
      yes,
    } => {
      let app_state = App::new(&cli.global)?;
      let mut ctx = crate::create::CreateContext {
        source_dir: app_state.source().to_path_buf(),
        target: app_state.target().to_path_buf(),
        dry_run: cli.global.dry_run,
        fold: app_state.resolved.cfg.fold,
        ignore_globs: app_state.resolved.cfg.ignore.clone(),
        conflict_resolver: app_state.make_resolver(),
      };
      let opts = crate::create::CreateOptions {
        app,
        path,
        verbose: app_state.resolved.cfg.verbose || cli.global.verbose > 0,
        interactive,
        empty,
        source,
        yes,
        force: cli.global.force,
      };
      crate::create::plan_create(&mut ctx, &opts)?;
      Ok(0)
    }
    Commands::Status {
      pkgs,
      short,
      json,
      states,
    } => {
      let app = App::new(&cli.global)?;
      let ignore = app.ignore()?;
      let pkgs = if pkgs.is_empty() {
        list_packages(app.source())?
      } else {
        pkgs
      };
      let mut packages = crate::status::compute_many(app.source(), app.target(), &pkgs, &ignore)?;
      if !states.is_empty() {
        packages.retain(|p| states.iter().any(|s| s.matches(p.status)));
      }
      let code = crate::status::exit_code(&packages);
      if short {
        println!(
          "{}",
          crate::status::render_short(&crate::status::summarize(&packages))
        );
      } else if json {
        println!("{}", crate::status::render_json(&packages)?);
      } else if cli.global.verbose > 0 {
        use std::io::IsTerminal;
        let no_color = !std::io::stdout().is_terminal() || std::env::var_os("NO_COLOR").is_some();
        println!(
          "{}",
          crate::status::render_verbose(&packages, no_color, cli.global.verbose > 1)
        );
      } else {
        use std::io::IsTerminal;
        let no_color = !std::io::stdout().is_terminal() || std::env::var_os("NO_COLOR").is_some();
        println!("{}", crate::status::render_table(&packages, no_color));
      }
      Ok(code)
    }
  }
}

pub fn main_entry() -> i32 {
  let cli = Cli::parse();
  match run(cli) {
    Ok(code) => code,
    Err(Error::Conflicts(report)) => {
      eprintln!("{report}");
      1
    }
    Err(Error::Aborted) => 2,
    Err(e) => {
      eprintln!("error: {e}");
      1
    }
  }
}
