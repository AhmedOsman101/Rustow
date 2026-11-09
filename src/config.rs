//! Configuration file model: `~/.config/rustow/config.toml`.
//!
//! Precedence: CLI flags > env vars (DOTFILES_DIR/DOTFILES_TARGET) > config.toml >
//! ~/.stowrc (only when no config.toml exists — see Task 3).

use std::fs;
use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// GNU Stow 2.4.1's default ignore list, verbatim from `default-ignore-list`.
/// Slashless patterns match path segments (basename); slashed patterns match
/// `/`-prefixed package-relative paths.
pub const STOW_DEFAULT_IGNORE: &[&str] = &[
  "RCS",
  ".+,v",
  "CVS",
  r"\.\#.+",
  r"\.cvsignore",
  r"\.svn",
  "_darcs",
  r"\.hg",
  ".git",
  ".gitignore",
  ".gitmodules",
  ".+~",
  r"\#.*\#",
  r"^/README.*",
  r"^/LICENSE.*",
  "^/COPYING",
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ConflictPolicy {
  /// Prompt on a TTY; abort when not a TTY.
  #[default]
  Prompt,
  /// Abort the whole run; never change anything.
  Abort,
  /// Leave the existing target in place.
  Skip,
  /// Delete the target; symlink to the package's version.
  Overwrite,
  /// Move the target's content into the package; symlink.
  Adopt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Config {
  /// Stow directory; its direct subdirectories are the packages.
  /// No default — rustow errors out until it is set.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub source: Option<PathBuf>,
  /// Link destination root; defaults to $HOME.
  #[serde(default = "default_target")]
  pub target: PathBuf,
  #[serde(default = "default_verbose")]
  pub verbose: bool,
  /// Fold package directories into single symlinks where possible.
  #[serde(default = "default_fold")]
  pub fold: bool,
  /// Extra glob patterns ignored on top of the built-in stow default list.
  #[serde(default = "default_ignore")]
  pub ignore: Vec<String>,
  /// Command template for the merge diff tool, e.g. "meld {left} {right}".
  /// Wins over the built-in registry and $EDITOR when set.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub diff_tool: Option<String>,
  /// What to do when a real file already occupies a target path.
  #[serde(default)]
  pub on_conflict: ConflictPolicy,
}

fn default_target() -> PathBuf {
  dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}
fn default_verbose() -> bool {
  true
}
fn default_fold() -> bool {
  true
}
fn default_ignore() -> Vec<String> {
  STOW_DEFAULT_IGNORE.iter().map(|s| s.to_string()).collect()
}

impl Default for Config {
  fn default() -> Self {
    Self {
      source: None,
      target: default_target(),
      verbose: default_verbose(),
      fold: default_fold(),
      ignore: default_ignore(),
      diff_tool: None,
      on_conflict: ConflictPolicy::Prompt,
    }
  }
}

/// `$XDG_CONFIG_HOME/rustow` (usually `~/.config/rustow`).
pub fn config_dir() -> PathBuf {
  dirs::config_dir()
    .unwrap_or_else(|| PathBuf::from(shellexpand::tilde("~/.config").into_owned()))
    .join("rustow")
}

/// `$XDG_CACHE_HOME/rustow` (usually `~/.cache/rustow`).
pub fn cache_dir() -> PathBuf {
  dirs::cache_dir()
    .unwrap_or_else(|| PathBuf::from(shellexpand::tilde("~/.cache").into_owned()))
    .join("rustow")
}

pub fn config_path() -> PathBuf {
  config_dir().join("config.toml")
}

pub fn stowrc_path() -> PathBuf {
  dirs::home_dir()
    .unwrap_or_else(|| PathBuf::from("."))
    .join(".stowrc")
}

/// Load `config.toml`. Returns `Ok(None)` when it does not exist.
pub fn load_file() -> Result<Option<Config>> {
  let path = config_path();
  if !path.exists() {
    return Ok(None);
  }
  let contents = fs::read_to_string(&path).map_err(Error::io(&path))?;
  let cfg: Config = toml::from_str(&contents)
    .map_err(|e| Error::Config(format!("failed to parse {}: {}", path.display(), e)))?;
  Ok(Some(cfg))
}

/// The annotated default configuration, printed by `config default` and written by `config init`.
pub const DEFAULT_CONFIG_TOML: &str = r#"# rustow — default configuration
# Docs: https://github.com/othman-moussa/rustow
# Precedence: CLI flags > env vars (DOTFILES_DIR, DOTFILES_TARGET) > this file.

# Stow directory; its direct subdirectories are the packages.
# REQUIRED — rustow refuses to run until it is set (no built-in default).
# source = "~/dotfiles/Configs"

# Link destination root. Default: $HOME.
# target = "~"

# Print what would happen and what is happening. Default: true.
verbose = true

# Fold package directories into single symlinks where possible
# (GNU stow's "folding"; false = always link individual files). Default: true.
fold = true

# Extra glob patterns ignored on top of the built-in GNU Stow 2.4.1 default list.
# Matched against package-relative paths with globset, e.g. "**/node_modules/**".
ignore = [
    "RCS",
    ".+,v",
    "CVS",
    "\\.\\#.+",
    "\\.cvsignore",
    "\\.svn",
    "_darcs",
    "\\.hg",
    ".git",
    ".gitignore",
    ".gitmodules",
    ".+~",
    "\\#.*\\#",
    "^/README.*",
    "^/LICENSE.*",
    "^/COPYING",
]

# Command template for the merge diff tool. Placeholders {left} and {right}
# are required. Wins over the built-in registry and $EDITOR.
# Examples: "meld {left} {right}", "nvim -d {left} {right}", "hx --new {left} {right}"
# diff_tool = "meld {left} {right}"

# What to do when a real file already occupies a target path:
#   prompt    — ask on a TTY (merge/overwrite/skip), abort otherwise [default]
#   abort     — never change anything; report and exit non-zero
#   skip      — leave the existing file in place
#   overwrite — target is deleted; symlink to the package's version
#   adopt     — target's content is moved into the package; symlink
on_conflict = "prompt"
"#;

/// Render the default configuration as a TOML string (parses as `Config`).
pub fn print_default() -> String {
  DEFAULT_CONFIG_TOML.to_string()
}

/// Write the default configuration to `path` (creating parent dirs).
/// Refuses to overwrite an existing file unless `force`.
pub fn write_default(path: &Path, force: bool) -> Result<PathBuf> {
  if path.exists() && !force {
    return Err(Error::Message(format!(
      "{} already exists (use --force to overwrite)",
      path.display()
    )));
  }
  if let Some(parent) = path.parent() {
    fs::create_dir_all(parent).map_err(Error::io(parent))?;
  }
  fs::write(path, print_default()).map_err(Error::io(path))?;
  Ok(path.to_path_buf())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn default_config_has_stow_list() {
    let cfg = Config::default();
    assert_eq!(cfg.ignore, default_ignore());
    assert_eq!(cfg.ignore.len(), 16);
    assert!(cfg.source.is_none());
    assert!(cfg.verbose);
    assert!(cfg.fold);
    assert_eq!(cfg.on_conflict, ConflictPolicy::Prompt);
  }

  #[test]
  fn default_toml_parses_and_matches_default() {
    let parsed: Config = toml::from_str(DEFAULT_CONFIG_TOML).unwrap();
    assert_eq!(parsed.ignore, default_ignore());
    assert_eq!(parsed.verbose, true);
    assert_eq!(parsed.fold, true);
    assert_eq!(parsed.on_conflict, ConflictPolicy::Prompt);
    assert!(parsed.source.is_none());
  }

  #[test]
  fn deny_unknown_fields_rejected() {
    let err = toml::from_str::<Config>("bogus_key = 1\n").unwrap_err();
    assert!(err.to_string().contains("unknown field"));
  }
}

// --- Task 3: stowrc fallback, precedence, provenance ----------------------

pub const ENV_DIR: &str = "DOTFILES_DIR";
pub const ENV_TARGET: &str = "DOTFILES_TARGET";

/// Where the effective configuration came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
  /// `config.toml` present — the normal case, no notice.
  File,
  /// A `--dir`/`--target` CLI flag won — effectively silent, no notice.
  Cli,
  /// No config file; `DOTFILES_DIR`/`DOTFILES_TARGET` (or `.stowrc`'s target fallback) in use.
  Env,
  /// No config file; `~/.stowrc` fallback in use.
  StowRc,
}

#[derive(Debug, Default, Clone)]
pub struct Stowrc {
  pub source: Option<PathBuf>,
  pub target: Option<PathBuf>,
  pub verbose: Option<bool>,
  pub warnings: Vec<String>,
}

fn expand_tilde_and_vars(s: &str) -> String {
  shellexpand::full(s)
    .map(|c| c.into_owned())
    .unwrap_or_else(|_| shellexpand::tilde(s).into_owned())
}

/// Parse the contents of a stowrc file (usually `~/.stowrc`).
///
/// GNU stow treats each line as a shell-ish argument list; we use shlex,
/// expand `$VAR`/`~`, and support `--dir`/`--target`/`--verbose`. Everything
/// else is collected as a warning and ignored.
pub fn parse_stowrc(contents: &str) -> Result<Stowrc> {
  let mut out = Stowrc::default();
  for (lineno, raw) in contents.lines().enumerate() {
    let line = raw.trim();
    if line.is_empty() || line.starts_with('#') {
      continue;
    }
    let tokens = shlex::split(line)
      .ok_or_else(|| Error::Config(format!("stowrc line {}: unmatched quotes", lineno + 1)))?;
    let mut i = 0;
    while i < tokens.len() {
      let tok = &tokens[i];
      let (key, value) = match tok.split_once('=') {
        Some((k, v)) => (k.to_string(), Some(v.to_string())),
        None => {
          if i + 1 < tokens.len() && !tokens[i + 1].starts_with('-') {
            (tok.clone(), Some(tokens[i + 1].clone()))
          } else {
            (tok.clone(), None)
          }
        }
      };
      let take_value = |i: &mut usize| -> Result<Option<String>> {
        match &value {
          Some(v) => Ok(Some(v.clone())),
          None => {
            *i += 1;
            Ok(tokens.get(*i).cloned())
          }
        }
      };
      match key.as_str() {
        "--dir" | "-d" => {
          if let Some(v) = take_value(&mut i)? {
            out.source = Some(PathBuf::from(expand_tilde_and_vars(&v)));
          } else {
            out
              .warnings
              .push(format!("stowrc line {}: --dir needs a value", lineno + 1));
          }
        }
        "--target" | "-t" => {
          if let Some(v) = take_value(&mut i)? {
            out.target = Some(PathBuf::from(expand_tilde_and_vars(&v)));
          } else {
            out.warnings.push(format!(
              "stowrc line {}: --target needs a value",
              lineno + 1
            ));
          }
        }
        "--verbose" | "-v" => {
          out.verbose = Some(match value.as_deref() {
            Some("0") | Some("false") | Some("no") => false,
            _ => true,
          });
          if value.is_none() {
            // consume nothing; value was inline flag
          }
        }
        "--ignore" => {
          out.warnings.push(format!(
            "stowrc line {}: --ignore is not supported by rustow (use `ignore` in config.toml)",
            lineno + 1
          ));
          if value.is_none() {
            i += 1; // swallow its value
          }
        }
        other => {
          out.warnings.push(format!(
            "stowrc line {}: ignoring unsupported option `{}`",
            lineno + 1,
            other
          ));
        }
      }
      i += 1;
    }
  }
  Ok(out)
}

/// CLI overrides (global flags). `None` = not given on the command line.
#[derive(Debug, Default, Clone)]
pub struct CliOverrides {
  pub dir: Option<PathBuf>,
  pub target: Option<PathBuf>,
}

/// The fully resolved configuration plus its provenance.
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
  pub cfg: Config,
  pub source: PathBuf,
  pub target: PathBuf,
  pub provenance: Provenance,
}

/// Resolve configuration: CLI flags > env vars (DOTFILES_DIR/DOTFILES_TARGET)
/// > config.toml > ~/.stowrc (only when no config.toml exists).
pub fn resolve(cli: &CliOverrides) -> Result<ResolvedConfig> {
  // 1. config.toml (authoritative when present)
  let file_cfg = load_file()?;
  let mut cfg = file_cfg.clone().unwrap_or_default();
  let mut target = cfg.target.clone();
  let mut provenance = Provenance::File;

  let mut source = cfg.source.clone();
  if file_cfg.is_none() {
    // 2a. env vars
    let env_dir = std::env::var_os(ENV_DIR).map(PathBuf::from);
    let env_target = std::env::var_os(ENV_TARGET).map(PathBuf::from);
    if let Some(d) = env_dir {
      source = Some(PathBuf::from(expand_tilde_and_vars(&d.to_string_lossy())));
      provenance = Provenance::Env;
    }
    if let Some(t) = env_target {
      target = PathBuf::from(expand_tilde_and_vars(&t.to_string_lossy()));
    }
    // 2b. stowrc — only when still unresolved (env wins; file absent)
    if source.is_none() {
      let stowrc = stowrc_path();
      if stowrc.exists() {
        let contents = fs::read_to_string(&stowrc).map_err(Error::io(&stowrc))?;
        let parsed = parse_stowrc(&contents)?;
        for w in parsed.warnings {
          eprintln!("rustow: warning: {w}");
        }
        if parsed.source.is_some() {
          source = parsed.source;
        }
        if let Some(t) = parsed.target {
          target = t;
        }
        if let Some(v) = parsed.verbose {
          cfg.verbose = v;
        }
        provenance = Provenance::StowRc;
      }
    }
  }

  // 3. CLI flags beat everything
  if let Some(d) = &cli.dir {
    source = Some(PathBuf::from(expand_tilde_and_vars(&d.to_string_lossy())));
    provenance = Provenance::Cli;
  }
  if let Some(t) = &cli.target {
    target = PathBuf::from(expand_tilde_and_vars(&t.to_string_lossy()));
  }

  // expand ~ on file-provided values
  let source = source.map(|s| PathBuf::from(expand_tilde_and_vars(&s.to_string_lossy())));
  let target = PathBuf::from(expand_tilde_and_vars(&target.to_string_lossy()));

  let source = source.ok_or(Error::SourceNotSet)?;
  Ok(ResolvedConfig {
    cfg,
    source,
    target,
    provenance,
  })
}

// --- Provenance notice ------------------------------------------------------

pub fn provenance_stamp_path() -> PathBuf {
  cache_dir().join("silence-provenance-notice")
}

pub fn provenance_silenced() -> bool {
  provenance_silenced_at(&provenance_stamp_path())
}

pub fn provenance_silenced_at(path: &Path) -> bool {
  path.exists()
}

pub fn silence_provenance_at(path: &Path) -> std::io::Result<()> {
  if let Some(parent) = path.parent() {
    fs::create_dir_all(parent)?;
  }
  fs::write(path, b"")
}

/// Permanently silence the provenance notice (best effort; warns on failure).
pub fn silence_provenance() {
  let stamp = provenance_stamp_path();
  if let Some(parent) = stamp.parent()
    && let Err(e) = fs::create_dir_all(parent).and_then(|_| fs::write(&stamp, b""))
  {
    eprintln!("rustow: warning: cannot write {}: {e}", stamp.display());
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoticeKind {
  /// config.toml present or silenced — no notice.
  None,
  /// Non-TTY — warn on stderr, continue.
  StderrOnly,
  /// TTY — ask the user.
  Prompt,
}

pub fn notice_kind(provenance: Provenance, silenced: bool, is_tty: bool) -> NoticeKind {
  match provenance {
    Provenance::File | Provenance::Cli => NoticeKind::None,
    _ if silenced => NoticeKind::None,
    _ if is_tty => NoticeKind::Prompt,
    _ => NoticeKind::StderrOnly,
  }
}

fn notice_description(resolved: &ResolvedConfig) -> String {
  match resolved.provenance {
    Provenance::StowRc => format!(
      "no config.toml found — using {} (stowrc fallback)",
      stowrc_path().display()
    ),
    Provenance::Env => format!(
      "no config.toml found — using env vars ({}={}, {}={})",
      ENV_DIR,
      resolved.source.display(),
      ENV_TARGET,
      resolved.target.display()
    ),
    Provenance::File => String::new(),
    Provenance::Cli => String::new(),
  }
}

/// Returns Ok(true) to continue, Ok(false)/Err to stop. On TTY without a
/// config file, asks the user what to do.
pub fn notice(resolved: &ResolvedConfig) -> Result<()> {
  use std::io::IsTerminal;
  let silenced = provenance_silenced();
  let is_tty = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
  match notice_kind(resolved.provenance, silenced, is_tty) {
    NoticeKind::None => Ok(()),
    NoticeKind::StderrOnly => {
      eprintln!("rustow: warning: {}", notice_description(resolved));
      eprintln!("rustow: run `rustow config init` to create one");
      Ok(())
    }
    NoticeKind::Prompt => {
      eprintln!("rustow: warning: {}", notice_description(resolved));
      match crate::prompt::confirm_provenance()? {
        crate::prompt::ProvenanceChoice::Create => {
          let path = config_path();
          crate::config::write_resolved(&path, resolved)?;
          println!("wrote {} — using it from now on", path.display());
          Ok(())
        }
        crate::prompt::ProvenanceChoice::Keep => Ok(()),
        crate::prompt::ProvenanceChoice::Silence => {
          silence_provenance();
          Ok(())
        }
        crate::prompt::ProvenanceChoice::Abort => Err(Error::Aborted),
      }
    }
  }
}

/// Write a config pre-filled from the values currently in use (used by the
/// provenance prompt's `create` choice).
pub fn write_resolved(path: &Path, resolved: &ResolvedConfig) -> Result<PathBuf> {
  fn toml_string(s: &str) -> String {
    let esc = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{esc}\"")
  }

  let mut s = String::from("# rustow — configuration (pre-filled from the environment in use)\n");
  s.push_str(&format!(
    "source = {}\n",
    toml_string(&resolved.source.to_string_lossy())
  ));
  s.push_str(&format!(
    "target = {}\n",
    toml_string(&resolved.target.to_string_lossy())
  ));
  s.push_str(&format!("verbose = {}\n", resolved.cfg.verbose));
  s.push_str(&format!("fold = {}\n\n", resolved.cfg.fold));
  if resolved.cfg.ignore != default_ignore() {
    s.push_str("ignore = [\n");
    for pat in &resolved.cfg.ignore {
      s.push_str(&format!("  {},\n", toml_string(pat)));
    }
    s.push_str("]\n");
  }
  if let Some(dt) = &resolved.cfg.diff_tool {
    s.push_str(&format!("diff_tool = {}\n", toml_string(dt)));
  }
  if resolved.cfg.on_conflict != ConflictPolicy::Prompt {
    s.push_str(&format!(
      "on_conflict = {}\n",
      toml_string(match resolved.cfg.on_conflict {
        ConflictPolicy::Prompt => "prompt",
        ConflictPolicy::Abort => "abort",
        ConflictPolicy::Skip => "skip",
        ConflictPolicy::Overwrite => "overwrite",
        ConflictPolicy::Adopt => "adopt",
      })
    ));
  }
  if let Some(parent) = path.parent() {
    fs::create_dir_all(parent).map_err(Error::io(parent))?;
  }
  fs::write(path, s).map_err(Error::io(path))?;
  Ok(path.to_path_buf())
}

#[cfg(test)]
mod task3_tests {
  use super::*;

  #[test]
  fn stowrc_basic() {
    let parsed =
      parse_stowrc("--dir=/home/u/dotfiles/Configs\n--target=/home/u\n--verbose=1\n").unwrap();
    assert_eq!(
      parsed.source.as_deref(),
      Some(std::path::Path::new("/home/u/dotfiles/Configs"))
    );
    assert_eq!(
      parsed.target.as_deref(),
      Some(std::path::Path::new("/home/u"))
    );
    assert_eq!(parsed.verbose, Some(true));
    assert!(parsed.warnings.is_empty());
  }

  #[test]
  fn stowrc_expands_home_var() {
    let home = std::env::var("HOME").unwrap();
    let parsed = parse_stowrc("--dir=$HOME/dotfiles/Configs\n").unwrap();
    assert_eq!(
      parsed.source,
      Some(PathBuf::from(format!("{home}/dotfiles/Configs")))
    );
  }

  #[test]
  fn stowrc_space_separated_and_comments() {
    let parsed = parse_stowrc("# comment\n\n--dir /tmp/pkgs\n").unwrap();
    assert_eq!(
      parsed.source.as_deref(),
      Some(std::path::Path::new("/tmp/pkgs"))
    );
  }

  #[test]
  fn stowrc_unknown_keys_warn_not_fail() {
    let parsed = parse_stowrc("--adopt\n--ignore=x\n").unwrap();
    assert_eq!(parsed.warnings.len(), 2);
  }

  #[test]
  fn stowrc_verbose_zero_is_false() {
    let parsed = parse_stowrc("--verbose=0\n").unwrap();
    assert_eq!(parsed.verbose, Some(false));
  }

  #[test]
  fn notice_kind_matrix() {
    assert_eq!(notice_kind(Provenance::File, true, true), NoticeKind::None);
    assert_eq!(
      notice_kind(Provenance::File, false, false),
      NoticeKind::None
    );
    assert_eq!(notice_kind(Provenance::Cli, false, true), NoticeKind::None);
    assert_eq!(
      notice_kind(Provenance::StowRc, true, true),
      NoticeKind::None
    );
    assert_eq!(notice_kind(Provenance::Env, true, false), NoticeKind::None);
    assert_eq!(
      notice_kind(Provenance::StowRc, false, true),
      NoticeKind::Prompt
    );
    assert_eq!(
      notice_kind(Provenance::Env, false, true),
      NoticeKind::Prompt
    );
    assert_eq!(
      notice_kind(Provenance::StowRc, false, false),
      NoticeKind::StderrOnly
    );
    assert_eq!(
      notice_kind(Provenance::Env, false, false),
      NoticeKind::StderrOnly
    );
  }

  #[test]
  fn stamp_silences() {
    let dir = tempfile::tempdir().unwrap();
    let stamp = dir.path().join("rustow/silence-provenance-notice");
    assert!(!provenance_silenced_at(&stamp));
    silence_provenance_at(&stamp).unwrap();
    assert!(provenance_silenced_at(&stamp));
  }
}
