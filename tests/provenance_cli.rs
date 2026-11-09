//! Provenance notice end-to-end tests (env vs stowrc fallback vs config file).

use std::fs;
use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::PredicateBooleanExt;
use predicates::str::contains;

struct Rig {
  _tmp: tempfile::TempDir,
  source: PathBuf,
  target: PathBuf,
  xdg_config: PathBuf,
  xdg_cache: PathBuf,
  home: PathBuf,
}

fn rig() -> Rig {
  let tmp = tempfile::tempdir().unwrap();
  let source = tmp.path().join("Configs");
  let target = tmp.path().join("home");
  let xdg_config = tmp.path().join("xdg-config");
  let xdg_cache = tmp.path().join("xdg-cache");
  let home = tmp.path().join("userhome");
  fs::create_dir_all(&source).unwrap();
  fs::create_dir_all(&target).unwrap();
  fs::create_dir_all(&xdg_config).unwrap();
  fs::create_dir_all(&xdg_cache).unwrap();
  fs::create_dir_all(&home).unwrap();
  Rig {
    _tmp: tmp,
    source,
    target,
    xdg_config,
    xdg_cache,
    home,
  }
}

impl Rig {
  fn cmd(&self) -> Command {
    let mut cmd = Command::cargo_bin("rustow").unwrap();
    cmd
      .env("XDG_CONFIG_HOME", &self.xdg_config)
      .env("XDG_CACHE_HOME", &self.xdg_cache)
      .env("HOME", &self.home)
      .env_remove("DOTFILES_DIR")
      .env_remove("DOTFILES_TARGET");
    cmd
  }

  fn make_pkg(&self, pkg: &str) {
    let p = self.source.join(pkg).join(".zshrc");
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, format!("contents of {pkg}")).unwrap();
  }
}

#[test]
fn env_only_invocation_warns_but_works() {
  let r = rig();
  r.make_pkg("zsh");
  r.cmd()
    .env("DOTFILES_DIR", &r.source)
    .env("DOTFILES_TARGET", &r.target)
    .args(["link", "zsh"])
    .assert()
    .success()
    .stderr(contains("no config.toml"))
    .stderr(contains("env vars"));
  assert!(r.target.join(".zshrc").is_symlink());
}

#[test]
fn silence_stamp_suppresses_warning() {
  let r = rig();
  r.make_pkg("zsh");
  let stamp = r.xdg_cache.join("rustow/silence-provenance-notice");
  fs::create_dir_all(stamp.parent().unwrap()).unwrap();
  fs::write(&stamp, b"").unwrap();
  r.cmd()
    .env("DOTFILES_DIR", &r.source)
    .env("DOTFILES_TARGET", &r.target)
    .args(["link", "zsh"])
    .assert()
    .success()
    .stderr(contains("no config.toml").not());
}

#[test]
fn stowrc_fallback_is_used_and_warns() {
  let r = rig();
  r.make_pkg("zsh");
  fs::write(
    r.home.join(".stowrc"),
    format!(
      "--dir={}\n--target={}\n--verbose=1\n",
      r.source.display(),
      r.target.display()
    ),
  )
  .unwrap();
  r.cmd()
    .args(["link", "zsh"])
    .assert()
    .success()
    .stderr(contains("stowrc"));
  assert!(r.target.join(".zshrc").is_symlink());
}

#[test]
fn config_toml_beats_stowrc_and_env_is_ignored_when_file_present() {
  let r = rig();
  r.make_pkg("zsh");
  // stowrc points elsewhere — must be ignored in favour of config.toml
  let other = r.home.join("elsewhere");
  fs::create_dir_all(&other).unwrap();
  fs::write(
    r.home.join(".stowrc"),
    format!("--dir={}\n--target={}\n", other.display(), other.display()),
  )
  .unwrap();
  let cfg_dir = r.xdg_config.join("rustow");
  fs::create_dir_all(&cfg_dir).unwrap();
  fs::write(
    cfg_dir.join("config.toml"),
    format!(
      "source = \"{}\"\ntarget = \"{}\"\nverbose = false\n",
      r.source.display(),
      r.target.display()
    ),
  )
  .unwrap();
  r.cmd()
    .args(["link", "zsh"])
    .assert()
    .success()
    .stderr(contains("stowrc").not())
    .stderr(contains("no config.toml").not());
  assert!(r.target.join(".zshrc").is_symlink());
}

#[test]
fn garbage_env_dir_errors_cleanly_not_panic() {
  let r = rig();
  let missing = r.home.join("does-not-exist");
  let out = r
    .cmd()
    .env("DOTFILES_DIR", &missing)
    .env("DOTFILES_TARGET", &r.target)
    .args(["sync"])
    .assert()
    .failure();
  let stderr = String::from_utf8(out.get_output().stderr.clone()).unwrap();
  assert!(
    stderr.contains("does-not-exist") || stderr.contains("error"),
    "stderr: {stderr}"
  );
}

#[test]
fn config_init_writes_config_and_silences_future_warnings() {
  let r = rig();
  r.make_pkg("zsh");
  // 1. env-only run: warning
  r.cmd()
    .env("DOTFILES_DIR", &r.source)
    .env("DOTFILES_TARGET", &r.target)
    .args(["link", "zsh"])
    .assert()
    .success()
    .stderr(contains("no config.toml"));
  // 2. write the default config manually, then the warning is gone forever
  let cfg_dir = r.xdg_config.join("rustow");
  fs::create_dir_all(&cfg_dir).unwrap();
  fs::write(
    cfg_dir.join("config.toml"),
    format!(
      "source = \"{}\"\ntarget = \"{}\"\nverbose = false\n",
      r.source.display(),
      r.target.display()
    ),
  )
  .unwrap();
  r.cmd()
    .args(["link", "zsh"])
    .assert()
    .success()
    .stderr(contains("no config.toml").not());
}

#[test]
fn config_init_creates_parseable_default() {
  let r = rig();
  r.cmd().args(["config", "init"]).assert().success();
  let written = r.xdg_config.join("rustow/config.toml");
  assert!(written.exists());
  let text = fs::read_to_string(&written).unwrap();
  let parsed: toml::Value = toml::from_str(&text).unwrap();
  assert_eq!(parsed.get("fold").unwrap().as_bool(), Some(true));
  assert_eq!(parsed.get("verbose").unwrap().as_bool(), Some(true));
  assert_eq!(parsed.get("on_conflict").unwrap().as_str(), Some("prompt"));
  assert!(parsed.get("ignore").unwrap().as_array().unwrap().len() >= 16);
}

#[test]
fn stowrc_var_expansion_works() {
  let r = rig();
  let env_src = r.home.join("ConfigsFromEnv").join("zsh/.config/zsh");
  fs::create_dir_all(&env_src).unwrap();
  fs::write(env_src.join("rc.zsh"), "x").unwrap();
  let env_target = r.home.join("TargetHome");
  fs::create_dir_all(&env_target).unwrap();
  fs::write(
    r.home.join(".stowrc"),
    "--dir=$HOME/ConfigsFromEnv\n--target=$HOME/TargetHome\n--verbose=1\n",
  )
  .unwrap();
  r.cmd()
    .env("HOME", &r.home)
    .args(["link", "zsh"])
    .assert()
    .success()
    .stderr(contains("stowrc"));
  assert!(env_target.join(".config/zsh/rc.zsh").exists());
}

#[test]
fn provenance_prompt_choice_create_prefills_config_file() {
  // Exercise write_resolved directly: simulates choosing [c]reate at the provenance prompt
  let tmp = tempfile::tempdir().unwrap();
  let source = tmp.path().join("Configs");
  let target = tmp.path().join("home");
  fs::create_dir_all(&source).unwrap();
  fs::create_dir_all(&target).unwrap();
  let resolved = rustow::config::ResolvedConfig {
    cfg: rustow::config::Config::default(),
    source: source.clone(),
    target: target.clone(),
    provenance: rustow::config::Provenance::StowRc,
  };
  let cfg_path = tmp.path().join("config.toml");
  let written = rustow::config::write_resolved(&cfg_path, &resolved).unwrap();
  let text = fs::read_to_string(&written).unwrap();
  assert!(text.contains(&format!("source = \"{}\"", source.display())));
  let parsed: toml::Value = toml::from_str(&text).unwrap();
  assert_eq!(
    parsed.get("source").unwrap().as_str().unwrap(),
    source.to_string_lossy()
  );
  assert_eq!(
    parsed.get("target").unwrap().as_str().unwrap(),
    target.to_string_lossy()
  );
}

#[test]
fn stowrc_unreadable_errors_not_panic() {
  let r = rig();
  r.make_pkg("zsh");
  // point HOME at a location where .stowrc is a directory — read fails for everyone
  let fake_home = r.home.join("fake");
  fs::create_dir_all(fake_home.join(".stowrc")).unwrap();
  r.cmd()
    .env("HOME", &fake_home)
    .args(["link", "zsh"])
    .assert()
    .failure();
}
