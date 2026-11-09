//! clap definitions and dispatch.

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

use crate::error::{Error, Result};

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

  /// Overwrite conflicting files without asking.
  #[arg(short = 'f', long, global = true, conflicts_with = "adopt")]
  pub force: bool,

  /// Move target files into the package, then link (stow --adopt).
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

pub fn run(cli: Cli) -> Result<i32> {
  match cli.command {
    Commands::Config(cmd) => match cmd {
      ConfigCommands::Default => {
        print!("{}", crate::config::print_default());
        Ok(0)
      }
      ConfigCommands::Init { force, path } => {
        let path = path.unwrap_or_else(crate::config::config_path);
        let written = crate::config::write_default(&path, force)?;
        println!("wrote {}", written.display());
        Ok(0)
      }
    },
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
    Err(e) => {
      eprintln!("error: {e}");
      1
    }
  }
}
