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
