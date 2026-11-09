//! Diff-tool detection and the interactive merge flow used for real-file conflicts.
//!
//! Resolution order: `diff_tool` from config (wins) → PATH-detected built-in
//! registry (cached choice in `$XDG_CACHE_HOME/rustow/diff-tool`) → `$EDITOR`
//! opening both files as buffers. No interactive selection on non-TTYs.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Error, Result};

pub struct DiffTool {
  /// Registry key, used for display and caching.
  pub name: &'static str,
  /// Command template; {left} and {right} placeholders are required.
  pub template: &'static str,
  /// If true, wait for the tool to exit; if false, spawn and wait for Enter.
  pub blocks: bool,
}

/// Built-in registry. Terminal tools block; GUI tools we cannot tell apart
/// block too unless they are known to return immediately (emacs).
pub const REGISTRY: &[DiffTool] = &[
  DiffTool {
    name: "code",
    template: "code --diff --wait {left} {right}",
    blocks: true,
  },
  DiffTool {
    name: "codium",
    template: "codium --diff --wait {left} {right}",
    blocks: true,
  },
  DiffTool {
    name: "cursor",
    template: "cursor --diff --wait {left} {right}",
    blocks: true,
  },
  DiffTool {
    name: "vscode",
    template: "vscode --diff --wait {left} {right}",
    blocks: true,
  },
  DiffTool {
    name: "zed",
    template: "zed --diff --wait {left} {right}",
    blocks: true,
  },
  DiffTool {
    name: "meld",
    template: "meld {left} {right}",
    blocks: true,
  },
  DiffTool {
    name: "kdiff3",
    template: "kdiff3 {left} {right}",
    blocks: true,
  },
  DiffTool {
    name: "diffuse",
    template: "diffuse {left} {right}",
    blocks: true,
  },
  DiffTool {
    name: "kompare",
    template: "kompare {left} {right}",
    blocks: true,
  },
  DiffTool {
    name: "diffmerge",
    template: "diffmerge {left} {right}",
    blocks: true,
  },
  DiffTool {
    name: "tkdiff",
    template: "tkdiff {left} {right}",
    blocks: true,
  },
  DiffTool {
    name: "xxdiff",
    template: "xxdiff {left} {right}",
    blocks: true,
  },
  DiffTool {
    name: "guiffy",
    template: "guiffy {left} {right}",
    blocks: true,
  },
  DiffTool {
    name: "bcompare",
    template: "bcompare {left} {right}",
    blocks: true,
  },
  DiffTool {
    name: "nvim",
    template: "nvim -d {left} {right}",
    blocks: true,
  },
  DiffTool {
    name: "vimdiff",
    template: "vimdiff {left} {right}",
    blocks: true,
  },
  DiffTool {
    name: "gvimdiff",
    template: "gvimdiff -f {left} {right}",
    blocks: true,
  },
  DiffTool {
    name: "vi",
    template: "vi -d {left} {right}",
    blocks: true,
  },
  DiffTool {
    name: "emacs",
    template: "emacs --eval \"(ediff-files \\\"{left}\\\" \\\"{right}\\\")\"",
    blocks: false,
  },
  DiffTool {
    name: "colordiff",
    template: "colordiff -u {left} {right}",
    blocks: true,
  },
  DiffTool {
    name: "delta",
    template: "delta {left} {right}",
    blocks: true,
  },
  DiffTool {
    name: "diff",
    template: "diff -u --color=always {left} {right}",
    blocks: true,
  },
];

/// Resolve a template like `"meld {left} {right}"` to a full argument vector.
/// Errors if placeholders are missing/duplicated or the template is malformed.
pub fn template_tokens(template: &str, left: &Path, right: &Path) -> Result<Vec<String>> {
  if !template.contains("{left}") || !template.contains("{right}") {
    return Err(Error::Config(
      "diff_tool template must contain both {left} and {right}".to_string(),
    ));
  }
  // exactly once each
  if template.matches("{left}").count() != 1 || template.matches("{right}").count() != 1 {
    return Err(Error::Config(
      "diff_tool template must contain {left} and {right} exactly once each".to_string(),
    ));
  }
  let raw = shlex::split(template)
    .ok_or_else(|| Error::Config("diff_tool template has unmatched quotes".to_string()))?;
  let mut args: Vec<String> = Vec::with_capacity(raw.len());
  for token in raw {
    let t = token
      .replace("{left}", &left.to_string_lossy())
      .replace("{right}", &right.to_string_lossy());
    args.push(t);
  }
  if args.is_empty() {
    return Err(Error::Config("diff_tool template is empty".to_string()));
  }
  Ok(args)
}

/// First token (program name) of a template.
pub fn template_program(template: &str) -> Result<String> {
  let args = shlex::split(template)
    .ok_or_else(|| Error::Config("diff_tool template has unmatched quotes".to_string()))?;
  args
    .into_iter()
    .next()
    .ok_or_else(|| Error::Config("diff_tool template is empty".to_string()))
}

/// Cheap `which`: find an executable file on PATH.
pub fn which(program: &str) -> Option<PathBuf> {
  let path_var = std::env::var_os("PATH")?;
  for dir in std::env::split_paths(&path_var) {
    let candidate = dir.join(program);
    if is_executable(&candidate) {
      return Some(candidate);
    }
  }
  None
}

fn is_executable(p: &Path) -> bool {
  if !p.is_file() {
    return false;
  }
  #[cfg(unix)]
  {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(p)
      .map(|m| m.permissions().mode() & 0o111 != 0)
      .unwrap_or(false)
  }
  #[cfg(not(unix))]
  {
    true
  }
}

/// Registry entries whose program exists on PATH.
pub fn available() -> Vec<&'static DiffTool> {
  REGISTRY
    .iter()
    .filter(|t| which(template_program(t.template).unwrap_or_default().as_str()).is_some())
    .collect()
}

pub fn cache_path() -> PathBuf {
  crate::config::cache_dir().join("diff-tool")
}

pub fn read_cached_tool() -> Option<&'static DiffTool> {
  read_cached_tool_from(&cache_path())
}

fn read_cached_tool_from(path: &Path) -> Option<&'static DiffTool> {
  let name = fs::read_to_string(path).ok()?.trim().to_string();
  REGISTRY
    .iter()
    .find(|t| t.name == name)
    .filter(|t| which(t.name).is_some())
}

pub fn cache_tool(name: &str) -> Result<()> {
  cache_tool_to(&cache_path(), name)
}

fn cache_tool_to(path: &Path, name: &str) -> Result<()> {
  if let Some(parent) = path.parent() {
    fs::create_dir_all(parent).map_err(Error::io(parent))?;
  }
  fs::write(path, name).map_err(Error::io(path))?;
  Ok(())
}

pub struct ChosenTool {
  /// Full command template (already substituted-free; run with left/right at exec time).
  pub template: String,
  pub blocks: bool,
  pub label: String,
}

/// Pick a tool: config > cached > $EDITOR (non-interactive) > interactive pick on a TTY.
pub fn choose_tool(config_template: Option<&str>, interactive: bool) -> Result<ChosenTool> {
  // 1. config wins
  if let Some(template) = config_template {
    let program = template_program(template)?;
    if which(&program).is_none() {
      return Err(Error::Config(format!(
        "diff_tool program not found on PATH: {program}"
      )));
    }
    // validate the template now, before the user is thrown into a diff view
    let tmp_left = PathBuf::from("/dev/null");
    let tmp_right = PathBuf::from("/dev/null");
    template_tokens(template, &tmp_left, &tmp_right)?;
    return Ok(ChosenTool {
      template: template.to_string(),
      blocks: true,
      label: program,
    });
  }

  // 2. cached choice
  if let Some(tool) = read_cached_tool() {
    return Ok(ChosenTool {
      template: tool.template.to_string(),
      blocks: tool.blocks,
      label: tool.name.to_string(),
    });
  }

  // 3. interactive pick on a TTY, else $EDITOR fallback
  if interactive {
    let tools = available();
    if !tools.is_empty() {
      let chosen = pick_interactive(&tools)?;
      let _ = cache_tool(chosen.name);
      return Ok(ChosenTool {
        template: chosen.template.to_string(),
        blocks: chosen.blocks,
        label: chosen.name.to_string(),
      });
    }
  }
  if let Ok(editor) = std::env::var("EDITOR") {
    let editor = editor.trim();
    if !editor.is_empty() {
      let program = template_program(editor)?;
      if which(&program).is_some() {
        return Ok(ChosenTool {
          template: format!("{editor} {{left}} {{right}}"),
          blocks: false,
          label: program,
        });
      }
    }
  }
  Err(Error::Message(
    "no diff tool available — set `diff_tool` in config.toml or $EDITOR".to_string(),
  ))
}

fn pick_interactive(tools: &[&'static DiffTool]) -> Result<&'static DiffTool> {
  use dialoguer::Select;
  use std::io::IsTerminal;

  if !std::io::stdin().is_terminal() {
    return Err(Error::Message(
      "interactive diff-tool selection needs a TTY; set `diff_tool` in config.toml".to_string(),
    ));
  }
  let labels: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
  let choice = Select::new()
    .with_prompt("no diff tool cached — pick one (used for future conflicts)")
    .items(&labels)
    .default(0)
    .interact()
    .map_err(|e| Error::Message(format!("prompt failed: {e}")))?;
  Ok(tools[choice])
}

/// Open the merge tool on (target vs package file). The user reconciles the
/// two into the PACKAGE file; afterwards the target is replaced with a symlink.
pub fn run_merge(tool: &ChosenTool, target: &Path, package_file: &Path) -> Result<()> {
  let args = template_tokens(&tool.template, target, package_file)?;
  let program = args[0].clone();
  eprintln!("the package file is the source of truth — reconcile your changes into:");
  eprintln!("  {}", package_file.display());
  let spawned = Command::new(&program)
    .args(&args[1..])
    .stdin(std::process::Stdio::inherit())
    .stdout(std::process::Stdio::inherit())
    .stderr(std::process::Stdio::inherit())
    .spawn()
    .map_err(|e| Error::Message(format!("failed to launch {program}: {e}")))?;
  if tool.blocks {
    spawned
      .wait_with_output()
      .map_err(|e| Error::Message(e.to_string()))?;
    return Ok(());
  }
  // GUI that returns immediately — tell the user to come back when done
  eprintln!("press Enter when you are done merging");
  let mut buf = String::new();
  std::io::stdin()
    .read_line(&mut buf)
    .map_err(|e| Error::Message(format!("prompt failed: {e}")))?;
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;

  fn dummy_paths() -> (PathBuf, PathBuf) {
    (
      PathBuf::from("/tmp/left file.txt"),
      PathBuf::from("/tmp/right.txt"),
    )
  }

  #[test]
  fn every_registry_template_is_valid() {
    let (l, r) = dummy_paths();
    for tool in REGISTRY {
      let args = template_tokens(tool.template, &l, &r)
        .unwrap_or_else(|e| panic!("registry template {} invalid: {e}", tool.name));
      assert_eq!(
        args.first().map(String::as_str),
        Some(template_program(tool.template).unwrap().as_str())
      );
      assert!(
        args.iter().any(|a| a.contains("/tmp/left file.txt")),
        "{} lost left",
        tool.name
      );
      assert!(
        args.iter().any(|a| a.contains("/tmp/right.txt")),
        "{} lost right",
        tool.name
      );
    }
  }

  #[test]
  fn template_requires_both_placeholders() {
    let (l, r) = dummy_paths();
    assert!(template_tokens("meld {left}", &l, &r).is_err());
    assert!(template_tokens("meld {right}", &l, &r).is_err());
    assert!(template_tokens("meld {left} {left}", &l, &r).is_err());
    assert!(template_tokens("meld {left} {right}", &l, &r).is_ok());
    assert!(template_tokens("meld 'unclosed", &l, &r).is_err());
    assert!(template_tokens("", &l, &r).is_err());
  }

  #[test]
  fn cache_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("rustow/diff-tool");
    assert!(read_cached_tool_from(&path).is_none());
    cache_tool_to(&path, "diff").unwrap();
    // "diff" is on every sane system
    if which("diff").is_some() {
      assert_eq!(read_cached_tool_from(&path).unwrap().name, "diff");
    }
  }

  #[test]
  fn config_template_wins_and_program_is_checked() {
    let chosen = choose_tool(Some("cp {left} {right}"), false).unwrap();
    assert_eq!(chosen.template, "cp {left} {right}");
    assert!(chosen.blocks);
    assert!(
      choose_tool(
        Some("definitely-not-a-real-program-xyz {left} {right}"),
        false
      )
      .is_err()
    );
    assert!(choose_tool(Some("cp {left}"), false).is_err());
  }

  #[test]
  fn run_merge_with_cp_stub_merges_into_package_file() {
    let tmp = tempfile::tempdir().unwrap();
    let target = tmp.path().join("target.txt");
    let package = tmp.path().join("package.txt");
    fs::write(&target, "user contents").unwrap();
    fs::write(&package, "package contents").unwrap();
    let tool = ChosenTool {
      template: "cp {left} {right}".to_string(),
      blocks: true,
      label: "cp".to_string(),
    };
    run_merge(&tool, &target, &package).unwrap();
    // merge ran the stub: package file now holds what the user had
    assert_eq!(fs::read_to_string(&package).unwrap(), "user contents");
  }

  #[test]
  fn default_editor_fallback_uses_editor_var() {
    // not checking actual behavior (env-dependent), just template construction logic
    let (l, r) = dummy_paths();
    let args = template_tokens("hx --new {left} {right}", &l, &r).unwrap();
    assert_eq!(
      args,
      vec!["hx", "--new", "/tmp/left file.txt", "/tmp/right.txt"]
    );
  }
}
