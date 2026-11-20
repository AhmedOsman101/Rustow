//! Configuration file model: `~/.config/rustow/config.toml`.
//!
//! Precedence: CLI flags > env vars (DOTFILES_DIR/DOTFILES_TARGET) > config.toml >
//! ~/.stowrc (only when no config.toml exists — see Task 3).

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use globset::GlobBuilder;
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

/// Settings for a single package, keyed by its directory name under `source`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PackageConfig {
  /// Biome-style list layered over the global `ignore` for this package only.
  /// Positive globs re-include paths the global list hid; `!`-prefixed globs
  /// hide them again. Absent means this package leaves the global list alone.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub includes: Option<Vec<String>>,
  /// Absolute destinations for package-relative source paths (ADR 0001).
  /// Keys are package-relative paths; values are absolute after `~` expansion.
  /// Trailing slashes are cosmetic and normalized away at load. The file-vs-
  /// directory kind is resolved from the source path's type at plan time.
  #[serde(
    default,
    skip_serializing_if = "Option::is_none",
    deserialize_with = "deserialize_root"
  )]
  pub root: Option<BTreeMap<String, String>>,
}

/// Normalize and validate one `[package.<name>.root]` mapping (ADR 0001).
///
/// - `key` is a package-relative source path. Absolute paths and `..`
///   components are rejected (the same rules `create --path` uses to keep a
///   path inside the package); cosmetic trailing slashes are stripped.
/// - `value` is an absolute destination. A leading `~` expands to `home`; a
///   value that is not absolute after expansion is rejected. Trailing slashes
///   are stripped by the lexical normalization.
///
/// `home` is passed in explicitly so the pure logic can be tested without
/// reading the real machine's HOME. Returns `(key, value)` normalized.
pub fn normalize_root_mapping(key: &str, value: &str, home: &Path) -> Result<(String, String)> {
  // Key: must stay inside the package (same rule as `create --path`).
  if Path::new(key).is_absolute() || key.split('/').any(|c| c == "..") {
    return Err(Error::Config(format!(
      "root key {key:?} escapes the package (absolute paths and '..' are not allowed)"
    )));
  }
  let key_norm = key
    .split('/')
    .filter(|c| !c.is_empty() && *c != ".")
    .collect::<Vec<_>>()
    .join("/");
  if key_norm.is_empty() {
    return Err(Error::Config(format!(
      "root key {key:?} is empty (needs at least one path component)"
    )));
  }

  // Value: expand a leading `~`, then require an absolute destination.
  let expanded = match value.strip_prefix('~') {
    Some(rest) if rest.is_empty() || rest.starts_with('/') => {
      format!("{}{rest}", home.to_string_lossy())
    }
    _ => value.to_string(),
  };
  if !Path::new(&expanded).is_absolute() {
    return Err(Error::Config(format!(
      "root key {key:?} has non-absolute destination {value:?} (after ~ expansion: {expanded:?})"
    )));
  }
  let value_norm = crate::link::lexical_normalize(Path::new(&expanded))
    .to_string_lossy()
    .into_owned();
  Ok((key_norm, value_norm))
}

/// Deserialize `[package.<name>.root]`, validating and normalizing every entry
/// as it is loaded so malformed keys/values fail at config load time.
fn deserialize_root<'de, D>(
  deserializer: D,
) -> std::result::Result<Option<BTreeMap<String, String>>, D::Error>
where
  D: serde::Deserializer<'de>,
{
  let raw = Option::<BTreeMap<String, String>>::deserialize(deserializer)?;
  let Some(raw) = raw else {
    return Ok(None);
  };
  let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("~"));
  let mut out = BTreeMap::new();
  for (key, value) in raw {
    let (key, value) =
      normalize_root_mapping(&key, &value, &home).map_err(serde::de::Error::custom)?;
    out.insert(key, value);
  }
  Ok(Some(out))
}

/// Compile one biome-style `includes` pattern.
///
/// A single leading `!` marks a re-exclusion; the remainder is a globset
/// pattern with the same slash semantics as the global `ignore` list. The
/// negation itself is interpreted when the effective matcher is built; this
/// function only validates that the pattern is well-formed.
pub fn compile_include_glob(pattern: &str) -> Result<globset::Glob> {
  let glob = pattern.strip_prefix('!').unwrap_or(pattern);
  GlobBuilder::new(glob)
    .literal_separator(true)
    .build()
    .map_err(|e| Error::Config(format!("invalid include glob {pattern:?}: {e}")))
}

/// Deserialize the `[package.<name>]` table, validating every `includes`
/// pattern as it is loaded so bad globs fail with the offending package key.
fn deserialize_package_configs<'de, D>(
  deserializer: D,
) -> std::result::Result<HashMap<String, PackageConfig>, D::Error>
where
  D: serde::Deserializer<'de>,
{
  let packages = HashMap::<String, PackageConfig>::deserialize(deserializer)?;
  for (name, package) in &packages {
    if let Some(includes) = &package.includes {
      for pattern in includes {
        compile_include_glob(pattern)
          .map_err(|e| serde::de::Error::custom(format!("package {name:?}: {e}")))?;
      }
    }
  }
  Ok(packages)
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
  #[schemars(default = "default_target_schema")]
  pub target: PathBuf,
  #[serde(default = "default_verbose")]
  pub verbose: bool,
  /// Fold package directories into single symlinks where possible.
  #[serde(default = "default_fold")]
  pub fold: bool,
  /// Packages in this list will never be folded (per-package `fold = false`).
  #[serde(default)]
  pub fold_ignore: Vec<String>,
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
  /// Elevation program for root-mapped actions (ADR 0001). When unset, rustow
  /// probes `sudo`, then `doas`. The program receives the action as its argv,
  /// e.g. `<program> ln -s <src> <dst>`.
  #[serde(default, skip_serializing_if = "Option::is_none")]
  pub elevate_cmd: Option<String>,
  /// Per-package settings, keyed by package name (`[package.<name>]`).
  #[serde(
    default,
    skip_serializing_if = "HashMap::is_empty",
    deserialize_with = "deserialize_package_configs"
  )]
  pub package: HashMap<String, PackageConfig>,
}

fn default_target() -> PathBuf {
  dirs::home_dir().unwrap_or_else(|| PathBuf::from("."))
}

/// Constant placeholder used only for the *schema*'s `default` (not at runtime).
fn default_target_schema() -> PathBuf {
  PathBuf::from("$HOME")
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
      fold_ignore: Vec::new(),
      ignore: default_ignore(),
      diff_tool: None,
      on_conflict: ConflictPolicy::Prompt,
      elevate_cmd: None,
      package: HashMap::new(),
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
///
/// The docs URL is embedded from Cargo.toml's `repository` field, so the generated
/// config cannot drift from the project's canonical repository.
pub const DEFAULT_CONFIG_TOML: &str = concat!(
  r#"# rustow — default configuration
# Docs: "#,
  env!("CARGO_PKG_REPOSITORY"),
  r#"
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

# Packages that must not be folded (default: empty). Use when a single app
# needs per-file links but you don't want to disable fold globally.
# fold_ignore = ["my-special-app"]

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

# Elevation program used ONLY for root-mapped actions (ADR 0001). It receives
# the action as argv, e.g. `<program> ln -s <src> <dst>`. Unset = probe `sudo`,
# then `doas`. Could be a path to a helper script (e.g. "/usr/local/bin/rustow-sudo").
# elevate_cmd = "sudo"

# Per-package settings. `includes` is a biome-style list layered over the
# global `ignore` for ONE package: positive globs re-include paths the global
# list hid, `!`-prefixed globs hide them again. Omitted = no change.
# [package.my-app]
# includes = ["**", "!**/*.local"]
#
# Root targeting (ADR 0001): map a package-relative source path to an absolute
# destination outside the target root (e.g. a file under /etc). Keys must stay
# inside the package (no `..`); values must be absolute after `~` expansion.
# A trailing `/` on either side is cosmetic. The source path's filesystem type
# decides the mapping kind at plan time: a directory root creates one symlink
# at <value>/<basename>, a file root is linked at <value> exactly. Root-mapped
# links may require elevation at apply time.
# [package.my-app.root]
# "configs/root" = "/etc/my-app/root"  # file → file
# "configs/"     = "/etc/my-app/"      # directory → directory
"#
);

/// Render the default configuration as a TOML string (parses as `Config`).
pub fn print_default() -> String {
  DEFAULT_CONFIG_TOML.to_string()
}

/// Write `text` to `path` (creating parent dirs).
/// Refuses to overwrite an existing file unless `force`.
fn write_config_text(path: &Path, force: bool, text: &str) -> Result<PathBuf> {
  if path.exists() && !force {
    return Err(Error::Message(format!(
      "{} already exists (use --force to overwrite)",
      path.display()
    )));
  }
  if let Some(parent) = path.parent() {
    fs::create_dir_all(parent).map_err(Error::io(parent))?;
  }
  fs::write(path, text).map_err(Error::io(path))?;
  Ok(path.to_path_buf())
}

/// Write the default configuration to `path` (creating parent dirs).
/// Refuses to overwrite an existing file unless `force`.
pub fn write_default(path: &Path, force: bool) -> Result<PathBuf> {
  write_config_text(path, force, &print_default())
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
    assert!(parsed.verbose);
    assert!(parsed.fold);
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

/// Expand `~` at the start of a token, and `$VAR` / `${VAR}` anywhere.
///
/// A backslash escapes the following character, so `\$HOME` stays the literal
/// `$HOME`. Unknown env vars stay literal (deliberate deviation from stow,
/// which aborts — rustow is more forgiving).
fn expand_tilde_and_vars(s: &str) -> String {
  fn push_tilde(out: &mut String) {
    if let Some(home) = dirs::home_dir() {
      out.push_str(&home.to_string_lossy());
    } else {
      out.push('~');
    }
  }

  let bytes = s.as_bytes();
  let mut out = String::with_capacity(s.len() + 16);
  let mut i = 0;
  while i < bytes.len() {
    match bytes[i] {
      b'\\' => {
        // backslash escapes the next code point, if any
        if let Some(c) = s[i + 1..].chars().next() {
          out.push(c);
          i += 1 + c.len_utf8();
        } else {
          out.push('\\');
          i += 1;
        }
      }
      b'$' => {
        if bytes.get(i + 1) == Some(&b'{') {
          // ${VAR}
          let mut j = i + 2;
          let start = j;
          while j < bytes.len() && bytes[j] != b'}' {
            j += 1;
          }
          if j >= bytes.len() {
            // unterminated ${ — literal
            out.push_str(&s[i..]);
            return out;
          }
          let var = &s[start..j];
          if var.is_empty() {
            out.push_str(&s[i..=j]);
          } else if let Ok(v) = std::env::var(var) {
            out.push_str(&v);
          } else {
            out.push_str(&s[i..=j]);
          }
          i = j + 1;
        } else {
          // $VAR
          let mut j = i + 1;
          while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
            j += 1;
          }
          if j == i + 1 {
            out.push('$');
            i += 1;
            continue;
          }
          let var = &s[i + 1..j];
          if let Ok(v) = std::env::var(var) {
            out.push_str(&v);
          } else {
            out.push('$');
            out.push_str(var);
          }
          i = j;
        }
      }
      b'~' if i == 0 => {
        push_tilde(&mut out);
        i += 1;
      }
      _ => {
        // copy whole UTF-8 codepoint
        let len = s[i..].chars().next().map(char::len_utf8).unwrap_or(1);
        out.push_str(&s[i..i + len]);
        i += len;
      }
    }
  }
  out
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
      let tok = tokens[i].clone();
      // inline `--key=value` form
      if let Some((k, v)) = tok.split_once('=') {
        match k {
          "--dir" | "-d" => out.source = Some(PathBuf::from(expand_tilde_and_vars(v))),
          "--target" | "-t" => out.target = Some(PathBuf::from(expand_tilde_and_vars(v))),
          "--verbose" | "-v" => out.verbose = Some(!matches!(v, "0" | "false" | "no")),
          "--ignore" => out.warnings.push(format!(
            "stowrc line {}: --ignore is not supported by rustow (use `ignore` in config.toml)",
            lineno + 1
          )),
          other => out.warnings.push(format!(
            "stowrc line {}: ignoring unsupported option `{}`",
            lineno + 1,
            other
          )),
        }
        i += 1;
        continue;
      }
      // separated `--key value` form: value is consumed here and i advances past it
      let next_value = |i: &usize| -> Option<String> {
        tokens.get(i + 1).filter(|t| !t.starts_with('-')).cloned()
      };
      match tok.as_str() {
        "--dir" | "-d" => {
          if let Some(v) = next_value(&i) {
            out.source = Some(PathBuf::from(expand_tilde_and_vars(&v)));
            i += 1; // consume the value
          } else {
            out
              .warnings
              .push(format!("stowrc line {}: --dir needs a value", lineno + 1));
          }
        }
        "--target" | "-t" => {
          if let Some(v) = next_value(&i) {
            out.target = Some(PathBuf::from(expand_tilde_and_vars(&v)));
            i += 1;
          } else {
            out.warnings.push(format!(
              "stowrc line {}: --target needs a value",
              lineno + 1
            ));
          }
        }
        "--verbose" | "-v" => {
          out.verbose = Some(true);
        }
        "--ignore" => {
          out.warnings.push(format!(
            "stowrc line {}: --ignore is not supported by rustow (use `ignore` in config.toml)",
            lineno + 1
          ));
          if next_value(&i).is_some() {
            i += 1; // swallow its value too
          }
        }
        "--adopt" | "--conflicts" | "--warn" | "--no-folding" | "--verbose-unstow"
        | "--dotfiles" | "--override" => {
          out.warnings.push(format!(
            "stowrc line {}: ignoring unsupported option `{tok}`",
            lineno + 1
          ));
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
  /// `--no-stowrc` / `RUSTOW_NO_STOWRC`: never read `~/.stowrc`, even as the fallback.
  pub no_stowrc: bool,
}

/// Environment kill-switch for the `~/.stowrc` fallback.
/// Truthy: 1, true, yes, on (case-insensitive). Everything else is false.
/// `pub(crate)` so the automatic `config init` adoption offer can honor it too.
pub(crate) fn env_no_stowrc() -> bool {
  match std::env::var("RUSTOW_NO_STOWRC") {
    Ok(v) => matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"),
    Err(_) => false,
  }
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
  // 1. config.toml (base layer when present)
  let file_cfg = load_file()?;
  let mut cfg = file_cfg.clone().unwrap_or_default();
  let mut target = cfg.target.clone();
  let mut provenance = Provenance::File;

  let mut source = cfg.source.clone();

  // 2. env vars layer on TOP of the file (spec: env > file > stowrc)
  let env_dir = std::env::var_os(ENV_DIR).map(PathBuf::from);
  let env_target = std::env::var_os(ENV_TARGET).map(PathBuf::from);
  if let Some(d) = env_dir {
    source = Some(PathBuf::from(expand_tilde_and_vars(&d.to_string_lossy())));
    provenance = Provenance::Env;
  }
  if let Some(t) = env_target {
    target = PathBuf::from(expand_tilde_and_vars(&t.to_string_lossy()));
    provenance = Provenance::Env;
  }

  // 2b. stowrc — only when no config file exists (env wins over it; spec: stowrc is a fallback)
  let stowrc_ignored = cli.no_stowrc || env_no_stowrc();
  if file_cfg.is_none() && source.is_none() {
    let stowrc = stowrc_path();
    if stowrc_ignored && stowrc.exists() {
      eprintln!(
        "rustow: note: ignoring {} (disabled by --no-stowrc / RUSTOW_NO_STOWRC)",
        stowrc.display()
      );
    }
    if !stowrc_ignored && stowrc.exists() {
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
      if provenance != Provenance::Env {
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
    provenance = Provenance::Cli;
  }

  // expand ~ on file-provided values, then lexically normalize (no `..` traversal)
  let source = source.map(|s| {
    crate::link::lexical_normalize(&PathBuf::from(expand_tilde_and_vars(&s.to_string_lossy())))
  });
  let target = crate::link::lexical_normalize(&PathBuf::from(expand_tilde_and_vars(
    &target.to_string_lossy(),
  )));

  let source = source.ok_or(Error::SourceNotSet)?;
  if source == target {
    return Err(Error::Message(format!(
      "source and target are the same directory ({}); refusing to link a package tree onto itself",
      source.display()
    )));
  }
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
  // A config file exists (possibly reached through a symlinked config dir):
  // env/CLI overrides may change provenance, but they never make the file
  // disappear. Never claim it is missing or offer to overwrite it.
  if config_path().exists() {
    return Ok(());
  }
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
  fn stowrc_single_backslash_dollar_expands() {
    let home = std::env::var("HOME").unwrap();
    // `\\$HOME` in the file -> shellwords consumes the backslash -> `$HOME/...`
    // -> expand_tilde_and_vars sees `$` not preceded by a backslash -> expands
    let parsed = parse_stowrc("--dir=\\$HOME/dotfiles/Configs\n").unwrap();
    assert_eq!(
      parsed.source,
      Some(PathBuf::from(format!("{home}/dotfiles/Configs")))
    );
  }

  #[test]
  fn stowrc_double_backslash_dollar_is_literal() {
    // `\\\\$HOME` in the file -> shellwords emits `\\$HOME` -> `\\$` parsed
    // as a literal `$`
    let parsed = parse_stowrc("--dir=\\\\$HOME/dotfiles/Configs\n").unwrap();
    assert_eq!(
      parsed.source.as_deref(),
      Some(std::path::Path::new("$HOME/dotfiles/Configs"))
    );
  }

  #[test]
  fn stowrc_escaped_brace_dollar_is_literal() {
    let parsed = parse_stowrc("--dir=\\\\${HOME}/dotfiles/Configs\n").unwrap();
    assert_eq!(
      parsed.source.as_deref(),
      Some(std::path::Path::new("${HOME}/dotfiles/Configs"))
    );
  }

  #[test]
  fn stowrc_tilde_expands() {
    let home = std::env::var("HOME").unwrap();
    let parsed = parse_stowrc("--dir=~/dotfiles/Configs\n").unwrap();
    assert_eq!(
      parsed.source,
      Some(PathBuf::from(format!("{home}/dotfiles/Configs")))
    );
  }

  #[test]
  fn stowrc_escaped_tilde_is_literal() {
    let parsed = parse_stowrc("--dir=\\\\~/dotfiles/Configs\n").unwrap();
    assert_eq!(
      parsed.source.as_deref(),
      Some(std::path::Path::new("~/dotfiles/Configs"))
    );
  }

  #[test]
  fn stowrc_escaped_non_ascii_codepoint() {
    // A doubled backslash in the file survives shlex as one literal `\`, so
    // expand_tilde_and_vars sees `\é` and must escape the whole code point.
    let parsed = parse_stowrc("--dir=\\\\é/dotfiles\n").unwrap();
    let source = parsed.source.unwrap();
    assert_eq!(source.to_str(), Some("é/dotfiles"));
    // Byte-wise escaping would yield Latin-1 "Ã" plus a stray "©".
    let shown = source.to_string_lossy();
    assert!(!shown.contains('Ã') && !shown.contains('©'));
  }

  #[test]
  fn stowrc_brace_form_expands() {
    let home = std::env::var("HOME").unwrap();
    let parsed = parse_stowrc("--dir=${HOME}/dotfiles/Configs\n").unwrap();
    assert_eq!(
      parsed.source,
      Some(PathBuf::from(format!("{home}/dotfiles/Configs")))
    );
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

// --- Task 21: shared stowrc adoption (ADR 0004) -----------------------------

/// One value that would be adopted from stowrc into config.toml.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrateAddition {
  /// Top-level config key: `source`, `target`, or `verbose`.
  pub key: &'static str,
  /// Human-readable value for the printed diff (unquoted).
  pub value: String,
  /// TOML literal written to the file (quoted string or boolean).
  pub toml: String,
}

/// A side-effect-free plan produced by [`migrate_stowrc`], reused by both
/// `config migrate` and `config init`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigratePlan {
  pub config_path: PathBuf,
  pub stowrc_path: PathBuf,
  /// Only the gaps: keys the config does not already set.
  pub additions: Vec<MigrateAddition>,
  /// Unsupported stowrc options, reported but never fatal.
  pub warnings: Vec<String>,
  /// Whether the destination config file existed when the plan was built.
  pub config_existed: bool,
}

impl MigratePlan {
  pub fn is_empty(&self) -> bool {
    self.additions.is_empty()
  }

  /// The adoption diff printed by `config migrate` (and `config init`).
  pub fn render_diff(&self) -> String {
    let mut s = String::from("stowrc -> config.toml (only values config leaves unset):\n");
    for a in &self.additions {
      s.push_str(&format!("  + {} = {}\n", a.key, a.value));
    }
    s
  }

  /// Write the adopted values into the config file, preserving everything
  /// already in it. Only top-level scalar keys are inserted, always before
  /// the first table header so they stay at the document root.
  pub fn apply(&self) -> Result<PathBuf> {
    let original = if self.config_existed && self.config_path.exists() {
      fs::read_to_string(&self.config_path).map_err(Error::io(&self.config_path))?
    } else {
      String::new()
    };
    let lines: Vec<String> = self
      .additions
      .iter()
      .map(|a| format!("{} = {}", a.key, a.toml))
      .collect();
    let merged = merge_top_level_keys(&original, &lines);
    if let Some(parent) = self.config_path.parent() {
      fs::create_dir_all(parent).map_err(Error::io(parent))?;
    }
    fs::write(&self.config_path, merged).map_err(Error::io(&self.config_path))?;
    Ok(self.config_path.clone())
  }
}

/// Quote a string as a full TOML basic string via toml's own serializer, so
/// control characters and other edge cases are escaped correctly (not just
/// backslashes and quotes).
fn quote_toml_string(s: &str) -> String {
  toml::Value::String(s.to_owned()).to_string()
}

/// In the migration path only, a stowrc warning that reports a dropped/malformed
/// value is fatal: ADR 0004 requires malformed stowrc to error cleanly with the
/// config untouched. Unsupported-option warnings stay non-fatal (they are the
/// runtime fallback's documented behavior and must not change here). The
/// warning text already begins `stowrc line N:`, so the error names both the
/// file and the line.
fn promote_malformed_stowrc_warnings(
  stowrc_path: &Path,
  warnings: &[String],
) -> Result<Vec<String>> {
  let mut kept = Vec::with_capacity(warnings.len());
  for w in warnings {
    if w.contains("needs a value") {
      return Err(Error::Config(format!(
        "malformed stowrc {}: {w}",
        stowrc_path.display()
      )));
    }
    kept.push(w.clone());
  }
  Ok(kept)
}

/// Which top-level keys are explicitly present in the config text. An empty
/// document has none. Parsing failures surface as a clean config error.
fn present_top_level_keys(text: &str) -> Result<std::collections::HashSet<String>> {
  if text.trim().is_empty() {
    return Ok(std::collections::HashSet::new());
  }
  let table: toml::Table =
    toml::from_str(text).map_err(|e| Error::Config(format!("failed to parse config.toml: {e}")))?;
  Ok(table.into_iter().map(|(k, _)| k).collect())
}

/// The adopted stow `source` must already be a directory. A stowrc that parses
/// but points at a moved/deleted stow dir must never become a config pointing
/// at nothing (Review Focus #2).
fn validate_migrate_source(path: &Path, key: &str) -> Result<()> {
  if !path.is_dir() {
    return Err(Error::Config(format!(
      "stowrc {key} path {} does not exist or is not a directory — refusing to write a config pointing at nothing",
      path.display()
    )));
  }
  Ok(())
}

/// The adopted `target` need not exist yet — rustow creates a missing target
/// root on link. It is only an error when the path exists but is not a
/// directory; a missing target is reported as a warning instead.
fn validate_migrate_target(path: &Path, warnings: &mut Vec<String>) -> Result<()> {
  if path.exists() && !path.is_dir() {
    return Err(Error::Config(format!(
      "stowrc target path {} exists but is not a directory — refusing to write a config pointing at a file",
      path.display()
    )));
  }
  if !path.exists() {
    warnings.push(format!(
      "stowrc target {} does not exist yet — rustow will create it on link",
      path.display()
    ));
  }
  Ok(())
}

/// Insert `lines` (complete `key = value` statements) at the document root of
/// `original`, preserving all existing text. They go before the first active
/// table header (a line starting with `[`), or at the end when there is none.
fn merge_top_level_keys(original: &str, lines: &[String]) -> String {
  if lines.is_empty() {
    return original.to_string();
  }
  let mut insertion = original.len();
  let mut offset = 0usize;
  for seg in original.split_inclusive('\n') {
    if seg.trim_start().starts_with('[') {
      insertion = offset;
      break;
    }
    offset += seg.len();
  }

  let mut out = String::new();
  out.push_str(&original[..insertion]);
  if insertion == original.len() && !original.is_empty() && !original.ends_with('\n') {
    out.push('\n');
  }
  for line in lines {
    out.push_str(line);
    out.push('\n');
  }
  if insertion < original.len() {
    out.push('\n');
  }
  out.push_str(&original[insertion..]);
  out
}

/// Build a migration plan from a stowrc file for a given destination config.
///
/// Reads and validates `stowrc_path`, then adopts only the values that the
/// config at `config_path` leaves UNSET (`source`, `target`, `verbose`).
/// Existing config values always win. The stowrc is never modified, and this
/// function never writes anything — callers print [`MigratePlan::render_diff`]
/// and, if the user agrees, [`MigratePlan::apply`].
pub fn migrate_stowrc(config_path: &Path, stowrc_path: &Path) -> Result<MigratePlan> {
  migrate_stowrc_inner(config_path, stowrc_path, false)
}

/// Build a plan as if `config_path` did not exist yet.
///
/// `config init --force` replaces the destination wholesale, so values in the
/// old file must not suppress adoption of the stowrc values. Shares all
/// validation and rendering with [`migrate_stowrc`]; nothing is written here
/// and the stowrc is never modified.
pub fn migrate_stowrc_for_fresh(config_path: &Path, stowrc_path: &Path) -> Result<MigratePlan> {
  migrate_stowrc_inner(config_path, stowrc_path, true)
}

fn migrate_stowrc_inner(
  config_path: &Path,
  stowrc_path: &Path,
  ignore_existing: bool,
) -> Result<MigratePlan> {
  let contents = fs::read_to_string(stowrc_path).map_err(Error::io(stowrc_path))?;
  let parsed = parse_stowrc(&contents)?;
  // Malformed values are fatal only here; the runtime fallback still warns.
  let mut warnings = promote_malformed_stowrc_warnings(stowrc_path, &parsed.warnings)?;

  let (config_text, config_existed) = if !ignore_existing && config_path.exists() {
    (
      fs::read_to_string(config_path).map_err(Error::io(config_path))?,
      true,
    )
  } else {
    (String::new(), false)
  };
  let present = present_top_level_keys(&config_text)?;

  let mut additions = Vec::new();
  if !present.contains("source")
    && let Some(src) = &parsed.source
  {
    validate_migrate_source(src, "source")?;
    additions.push(MigrateAddition {
      key: "source",
      value: src.to_string_lossy().into_owned(),
      toml: quote_toml_string(&src.to_string_lossy()),
    });
  }
  if !present.contains("target")
    && let Some(tgt) = &parsed.target
  {
    validate_migrate_target(tgt, &mut warnings)?;
    additions.push(MigrateAddition {
      key: "target",
      value: tgt.to_string_lossy().into_owned(),
      toml: quote_toml_string(&tgt.to_string_lossy()),
    });
  }
  if !present.contains("verbose")
    && let Some(v) = parsed.verbose
  {
    additions.push(MigrateAddition {
      key: "verbose",
      value: v.to_string(),
      toml: v.to_string(),
    });
  }

  Ok(MigratePlan {
    config_path: config_path.to_path_buf(),
    stowrc_path: stowrc_path.to_path_buf(),
    additions,
    warnings,
    config_existed,
  })
}

#[cfg(test)]
mod migrate_tests {
  use super::*;

  #[test]
  fn merge_inserts_before_table_and_preserves_existing_text() {
    let original = "# my config\nsource = \"/keep\"\n\n[package.foo]\nincludes = [\"**\"]\n";
    let lines = vec![
      "target = \"/new\"".to_string(),
      "verbose = false".to_string(),
    ];
    let merged = merge_top_level_keys(original, &lines);
    let parsed: toml::Table = toml::from_str(&merged).unwrap();
    assert_eq!(parsed.get("source").unwrap().as_str(), Some("/keep"));
    assert_eq!(parsed.get("target").unwrap().as_str(), Some("/new"));
    assert_eq!(parsed.get("verbose").unwrap().as_bool(), Some(false));
    assert!(merged.find("target = ").unwrap() < merged.find("[package.foo]").unwrap());
    assert!(merged.starts_with("# my config"));
  }

  #[test]
  fn merge_appends_to_empty_document() {
    let merged = merge_top_level_keys("", &["source = \"/new\"".to_string()]);
    assert_eq!(merged, "source = \"/new\"\n");
  }

  #[test]
  fn quote_toml_string_uses_full_toml_escaping() {
    let s = "C:\\a\"b\nc\td";
    let quoted = quote_toml_string(s);
    let table: toml::Table = toml::from_str(&format!("path = {quoted}\n")).unwrap();
    assert_eq!(table.get("path").unwrap().as_str(), Some(s));
  }

  #[test]
  fn malformed_value_warnings_are_fatal_but_unsupported_options_are_kept() {
    let path = std::path::Path::new("/nonexistent/.stowrc");
    let err =
      promote_malformed_stowrc_warnings(path, &["stowrc line 7: --dir needs a value".to_string()])
        .unwrap_err();
    let msg = err.to_string();
    assert!(
      msg.contains("/nonexistent/.stowrc") && msg.contains("line 7"),
      "{msg}"
    );

    let kept = promote_malformed_stowrc_warnings(
      path,
      &["stowrc line 2: ignoring unsupported option `--adopt`".to_string()],
    )
    .unwrap();
    assert_eq!(kept.len(), 1);
  }
}

// --- Task 22: inline adoption offer in `config init` (ADR 0004) -------------

/// The annotated default config with a plan's adopted stowrc values merged in.
///
/// `source` and `target` are commented out in the default template, so they
/// become active lines in place; `verbose` is active there, so it is replaced
/// in place. The surrounding documentation is preserved and no key is
/// duplicated. This is the file written when `config init` adopts a stowrc.
pub fn default_with_adopted(plan: &MigratePlan) -> String {
  let mut text = print_default();
  for addition in &plan.additions {
    let active = format!("{} = {}", addition.key, addition.toml);
    let mut replaced = false;
    let mut out = String::with_capacity(text.len() + active.len());
    for seg in text.split_inclusive('\n') {
      let trimmed = seg.trim_start();
      // Match both the commented-out placeholders and active keys, but never
      // prose that merely mentions a key name mid-sentence.
      let body = trimmed.strip_prefix('#').unwrap_or(trimmed).trim_start();
      let is_key = body
        .strip_prefix(addition.key)
        .is_some_and(|rest| rest.trim_start().starts_with('='));
      if is_key {
        out.push_str(&active);
        out.push('\n');
        replaced = true;
      } else {
        out.push_str(seg);
      }
    }
    text = out;
    if !replaced {
      text = merge_top_level_keys(&text, &[active]);
    }
  }
  text
}

/// Write the documented default with a plan's adopted stowrc values. Same
/// overwrite rules as [`write_default`]; the stowrc itself is never touched.
pub fn write_default_with_adopted(path: &Path, force: bool, plan: &MigratePlan) -> Result<PathBuf> {
  write_config_text(path, force, &default_with_adopted(plan))
}
