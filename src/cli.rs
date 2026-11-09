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

  /// Verbose output.
  #[arg(short, long, global = true)]
  pub verbose: bool,

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
    #[arg(short, long)]
    short: bool,
    /// Print machine-readable JSON.
    #[arg(long)]
    json: bool,
  },
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
    if item.file_type().map(|t| t.is_dir()).unwrap_or(false) {
      if let Some(name) = item.file_name().to_str() {
        out.push(name.to_string());
      }
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
          Box::new(PromptResolver)
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
        if self.resolved.cfg.verbose || self.globals.verbose {
          for plan in &plans {
            for action in &plan.actions {
              println!("{}", render_action(action));
            }
          }
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
    let n = link::apply_unlink(&plan, self.target(), self.source(), self.globals.dry_run)?;
    let verb = if self.globals.dry_run {
      "would unlink"
    } else {
      "unlinked"
    };
    if self.resolved.cfg.verbose || self.globals.verbose {
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

/// Interactive per-conflict prompt (TTY). The merge choice lands with the
/// diff-tool work; until then files offer skip/overwrite/adopt/abort.
struct PromptResolver;

impl ConflictResolver for PromptResolver {
  fn resolve(&mut self, conflict: &link::Conflict) -> Result<PromptOutcome> {
    use dialoguer::Select;
    use std::io::IsTerminal;

    if !std::io::stdin().is_terminal() {
      return Ok(PromptOutcome::Abort);
    }
    eprintln!("conflict: {conflict}");
    let items = [
      "skip (leave it as it is)",
      "overwrite (replace it with the package's version)",
      "adopt (move it into the package, then link)",
      "abort everything",
    ];
    let choice = Select::new()
      .with_prompt("what would you like to do?")
      .items(&items)
      .default(0)
      .interact()
      .map_err(|e| Error::Message(format!("prompt failed: {e}")))?;
    Ok(match choice {
      0 => PromptOutcome::Skip,
      1 => PromptOutcome::Overwrite,
      2 => PromptOutcome::Adopt,
      _ => PromptOutcome::Abort,
    })
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
    Commands::Status { pkgs, short, json } => {
      let app = App::new(&cli.global)?;
      let ignore = app.ignore()?;
      let pkgs = if pkgs.is_empty() {
        list_packages(app.source())?
      } else {
        pkgs
      };
      let packages = crate::status::compute_many(app.source(), app.target(), &pkgs, &ignore)?;
      let code = crate::status::exit_code(&packages);
      if short {
        println!(
          "{}",
          crate::status::render_short(&crate::status::summarize(&packages))
        );
      } else if json {
        println!("{}", crate::status::render_json(&packages)?);
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
