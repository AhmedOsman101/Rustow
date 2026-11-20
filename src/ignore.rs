//! Ignore handling: GNU Stow 2.4.1's built-in default list plus user globs
//! from `config.toml`. Nothing else is ignored.

use std::collections::HashMap;
use std::path::Path;

use globset::{GlobBuilder, GlobMatcher, GlobSet, GlobSetBuilder};
use regex::Regex;

use crate::config::{PackageConfig, STOW_DEFAULT_IGNORE, compile_include_glob};
use crate::error::{Error, Result};

/// One compiled biome-style `includes` entry. A leading `!` marks a
/// re-exclusion; the matcher itself has already had that prefix stripped by
/// `compile_include_glob`.
struct CompiledInclude {
  negated: bool,
  matcher: GlobMatcher,
}

pub struct Ignore {
  /// Slashed built-in regexps, matched against `/`-prefixed package-relative path.
  path_regexps: Vec<Regex>,
  /// Slashless built-in regexps (anchored `^…$`), matched against basename.
  segment_regexps: Vec<Regex>,
  /// User globs containing `/`, matched against the package-relative path.
  user_path_globs: Option<GlobSet>,
  /// User globs without `/`, matched against the basename.
  user_segment_globs: Option<GlobSet>,
  /// Per-package biome-style `includes`, layered over the rules above (ADR 0002).
  package_includes: HashMap<String, Vec<CompiledInclude>>,
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
      package_includes: HashMap::new(),
    })
  }

  /// Build the global rules plus each package's `includes` list. A package
  /// without `includes` keeps the global behavior unchanged.
  pub fn with_packages(
    user_globs: &[String],
    packages: &HashMap<String, PackageConfig>,
  ) -> Result<Self> {
    let mut ignore = Self::new(user_globs)?;
    for (pkg, cfg) in packages {
      let Some(includes) = &cfg.includes else {
        continue;
      };
      let mut compiled = Vec::with_capacity(includes.len());
      for pattern in includes {
        let negated = pattern.starts_with('!');
        let matcher = compile_include_glob(pattern)?.compile_matcher();
        compiled.push(CompiledInclude { negated, matcher });
      }
      ignore.package_includes.insert(pkg.clone(), compiled);
    }
    Ok(ignore)
  }

  /// Check a package-relative path (e.g. `.git/config`) against the global
  /// rules only. Callers that know the package should prefer `is_ignored_for`.
  pub fn is_ignored(&self, pkg_rel: &Path) -> bool {
    match normalized(pkg_rel) {
      Some(s) => self.globally_ignored(&s),
      None => false,
    }
  }

  /// Effective check for one package: built-in defaults + global `ignore`
  /// first, then that package's `includes` on top. Entries are applied in
  /// order and the last match wins — a positive glob re-includes a globally
  /// hidden path, a `!`-prefixed glob hides it again (ADR 0002). Without a
  /// list for this package the global verdict stands.
  pub fn is_ignored_for(&self, pkg: &str, pkg_rel: &Path) -> bool {
    let Some(s) = normalized(pkg_rel) else {
      return false;
    };
    let Some(includes) = self.package_includes.get(pkg) else {
      return self.globally_ignored(&s);
    };
    let mut hidden = self.globally_ignored(&s);
    for entry in includes {
      if entry.matcher.is_match(&s) {
        hidden = entry.negated;
      }
    }
    hidden
  }

  fn globally_ignored(&self, s: &str) -> bool {
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

/// Package-relative path as a trimmed, `/`-joined string; `None` for the root.
fn normalized(pkg_rel: &Path) -> Option<String> {
  let s = pkg_rel.to_string_lossy();
  let trimmed = s.trim_start_matches('/');
  (!trimmed.is_empty()).then(|| trimmed.to_string())
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
    assert!(ig.is_ignored(p("a/b/c.hgtags.x,v").as_path()));
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

  fn packages(includes: Option<Vec<&str>>) -> HashMap<String, PackageConfig> {
    let mut map = HashMap::new();
    map.insert(
      "alpha".to_string(),
      PackageConfig {
        includes: includes.map(|v| v.into_iter().map(str::to_string).collect()),
      },
    );
    map
  }

  #[test]
  fn package_includes_layer_over_global_and_stay_local() {
    let ig =
      Ignore::with_packages(&["*.bak".into()], &packages(Some(vec!["**", "!keep.bak"]))).unwrap();
    // global hide, then ** re-includes, then ! re-hides for alpha only
    assert!(ig.is_ignored_for("alpha", &p("keep.bak")));
    assert!(!ig.is_ignored_for("alpha", &p("other.bak")));
    // beta has no entry: global rules unchanged
    assert!(ig.is_ignored_for("beta", &p("other.bak")));
  }

  #[test]
  fn package_without_includes_matches_global() {
    let ig = Ignore::with_packages(&["*.bak".into()], &packages(None)).unwrap();
    assert!(ig.is_ignored_for("alpha", &p("other.bak")));
    assert!(!ig.is_ignored_for("alpha", &p("normal.txt")));
  }

  #[test]
  fn catch_all_negation_hides_everything_for_that_package() {
    let ig = Ignore::with_packages(&[], &packages(Some(vec!["!**/*"]))).unwrap();
    assert!(ig.is_ignored_for("alpha", &p("theme.yml")));
    assert!(ig.is_ignored_for("alpha", &p(".config/eza/theme.yml")));
    assert!(!ig.is_ignored_for("beta", &p("theme.yml")));
  }
}
