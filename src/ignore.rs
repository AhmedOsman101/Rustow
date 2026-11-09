//! Ignore handling: GNU Stow 2.4.1's built-in default list plus user globs
//! from `config.toml`. Nothing else is ignored.

use std::path::Path;

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use regex::Regex;

use crate::config::STOW_DEFAULT_IGNORE;
use crate::error::{Error, Result};

pub struct Ignore {
  /// Slashed built-in regexps, matched against `/`-prefixed package-relative path.
  path_regexps: Vec<Regex>,
  /// Slashless built-in regexps (anchored `^…$`), matched against basename.
  segment_regexps: Vec<Regex>,
  /// User globs containing `/`, matched against the package-relative path.
  user_path_globs: Option<GlobSet>,
  /// User globs without `/`, matched against the basename.
  user_segment_globs: Option<GlobSet>,
}

impl Ignore {
  pub fn new(user_globs: &[String]) -> Result<Self> {
    let mut path_regexps = Vec::new();
    let mut segment_regexps = Vec::new();
    for pat in STOW_DEFAULT_IGNORE {
      match Regex::new(pat) {
        Ok(re) => {
          if pat.contains('/') {
            path_regexps.push(re);
          } else {
            // stow anchors slashless patterns against the basename
            segment_regexps
              .push(Regex::new(&format!("^({pat})$")).map_err(|e| {
                Error::Config(format!("invalid default ignore pattern {pat}: {e}"))
              })?);
          }
        }
        Err(e) => {
          return Err(Error::Config(format!(
            "invalid default ignore pattern {pat}: {e}"
          )));
        }
      }
    }

    let mut path_builder = GlobSetBuilder::new();
    let mut segment_builder = GlobSetBuilder::new();
    let mut path_any = false;
    let mut segment_any = false;
    for glob in user_globs {
      let parsed = GlobBuilder::new(glob)
        .literal_separator(true)
        .build()
        .map_err(|e| Error::Config(format!("invalid ignore glob {glob:?}: {e}")))?;
      if glob.contains('/') {
        path_builder.add(parsed);
        path_any = true;
      } else {
        segment_builder.add(parsed);
        segment_any = true;
      }
    }

    Ok(Self {
      path_regexps,
      segment_regexps,
      user_path_globs: path_any
        .then(|| path_builder.build())
        .transpose()
        .map_err(|e| Error::Config(e.to_string()))?,
      user_segment_globs: segment_any
        .then(|| segment_builder.build())
        .transpose()
        .map_err(|e| Error::Config(e.to_string()))?,
    })
  }

  /// Check a package-relative path (e.g. `.git/config`).
  pub fn is_ignored(&self, pkg_rel: &Path) -> bool {
    let s = pkg_rel.to_string_lossy();
    let s = s.trim_start_matches('/');
    if s.is_empty() {
      return false;
    }
    // built-in slashed patterns: match `/`-prefixed path
    let prefixed = format!("/{s}");
    for re in &self.path_regexps {
      if re.is_match(&prefixed) {
        return true;
      }
    }
    // built-in slashless patterns: match ANY path segment
    // (a `.git` directory prunes its whole subtree, mirroring stow's walk)
    for seg in s.split('/') {
      for re in &self.segment_regexps {
        if re.is_match(seg) {
          return true;
        }
      }
    }
    // user globs
    if let Some(g) = &self.user_path_globs
      && g.is_match(s)
    {
      return true;
    }
    if let Some(g) = &self.user_segment_globs {
      for seg in s.split('/') {
        if g.is_match(seg) {
          return true;
        }
      }
    }
    false
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::path::PathBuf;

  fn ignore() -> Ignore {
    Ignore::new(&[]).unwrap()
  }

  fn p(s: &str) -> PathBuf {
    PathBuf::from(s)
  }

  #[test]
  fn stow_defaults() {
    let ig = ignore();
    assert!(ig.is_ignored(&p(".git/config")));
    assert!(ig.is_ignored(&p(".git")));
    assert!(ig.is_ignored(&p("sub/.git")));
    assert!(ig.is_ignored(&p("README.md")));
    assert!(ig.is_ignored(&p("LICENSE")));
    assert!(ig.is_ignored(&p("COPYING")));
    assert!(!ig.is_ignored(&p("sub/README.md")));
    assert!(ig.is_ignored(&p("dir/backup~")));
    assert!(ig.is_ignored(&p(".svn/entries")));
    assert!(ig.is_ignored(&p("_darcs/hashed")));
    assert!(ig.is_ignored(&p("theme.ple,v")));
    assert!(ig.is_ignored(&p("a/b/c.hgtags.x,v").as_path()));
    assert!(!ig.is_ignored(&p(".config/nvim/init.lua")));
    assert!(!ig.is_ignored(&p(".config/eza/theme.yml")));
  }

  #[test]
  fn user_globs() {
    let ig = Ignore::new(&[
      "**/node_modules/**".into(),
      "vscode/globalStorage/**".into(),
      "*.pyc".into(),
    ])
    .unwrap();
    assert!(ig.is_ignored(&p("pkg/node_modules/x/index.js")));
    assert!(ig.is_ignored(&p("vscode/globalStorage/state.vscdb")));
    assert!(ig.is_ignored(&p("sub/dir/file.pyc")));
    assert!(!ig.is_ignored(&p("src/main.py")));
    assert!(!ig.is_ignored(&p("vscode/settings.json")));
  }

  #[test]
  fn invalid_glob_errors() {
    assert!(Ignore::new(&["[unclosed".into()]).is_err());
  }
}
