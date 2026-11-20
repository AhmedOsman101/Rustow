//! Interactive conflict resolution driven through a real pseudo-terminal.
//!
//! `dialoguer::Select` only prompts when stdin is a TTY, so a piped-stdin
//! integration test cannot reach the overwrite/skip choices at all. These
//! tests spawn the real binary attached to a PTY, wait for the conflict select
//! to render, then send arrow keys + Enter to pick an option. Every fixture
//! isolates HOME / XDG_* / DOTFILES_* under `/tmp/opencode`; the real dotfiles
//! are never touched. The child is always driven to completion and reaped
//! here; an RAII guard also cleans up this fixture's own child if a panic
//! unwinds first — no signals to any other process, no stray processes.

use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{Child, CommandBuilder, PtySize, native_pty_system};

/// Owns exactly one child spawned by `Rig::run_pty` plus its output reader.
/// Normal completion calls `reap` (which clears `child`); on unwind `Drop`
/// kills and waits only that child, then joins the reader thread. No other
/// process is ever signalled by this test binary.
struct ChildGuard {
  child: Option<Box<dyn Child + Send + Sync>>,
  reader: Option<thread::JoinHandle<()>>,
}

impl ChildGuard {
  fn new(child: Box<dyn Child + Send + Sync>) -> Self {
    Self {
      child: Some(child),
      reader: None,
    }
  }

  fn attach_reader(&mut self, reader: thread::JoinHandle<()>) {
    self.reader = Some(reader);
  }

  /// Normal path: wait for the child and mark it reaped so `Drop` is a no-op.
  fn reap(&mut self) -> u32 {
    let code = self
      .child
      .as_mut()
      .expect("child already reaped")
      .wait()
      .unwrap()
      .exit_code();
    self.child = None;
    code
  }

  fn join_reader(&mut self) {
    if let Some(reader) = self.reader.take() {
      reader.join().unwrap();
    }
  }
}

impl Drop for ChildGuard {
  fn drop(&mut self) {
    if let Some(child) = self.child.as_mut() {
      // Unwind path: this is the child this fixture spawned and never reaped.
      if matches!(child.try_wait(), Ok(None)) {
        let _ = child.kill();
      }
      let _ = child.wait();
    }
    if let Some(reader) = self.reader.take() {
      let _ = reader.join();
    }
  }
}

struct Rig {
  _tmp: tempfile::TempDir,
  source: PathBuf,
  target: PathBuf,
  xdg: PathBuf,
  home: PathBuf,
}

fn rig() -> Rig {
  let tmp = tempfile::Builder::new()
    .prefix("rustow-pty-")
    .tempdir_in("/tmp/opencode")
    .unwrap();
  let source = tmp.path().join("Configs");
  let target = tmp.path().join("home");
  let xdg = tmp.path().join("xdg");
  let home = tmp.path().join("userhome");
  for dir in [&source, &target, &xdg, &home] {
    fs::create_dir_all(dir).unwrap();
  }
  Rig {
    _tmp: tmp,
    source,
    target,
    xdg,
    home,
  }
}

impl Rig {
  /// A real config file keeps the provenance notice out of the PTY session so
  /// the only select reached is the conflict prompt under test.
  fn write_config(&self) {
    let dir = self.xdg.join("rustow");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
      dir.join("config.toml"),
      format!(
        "source = \"{}\"\ntarget = \"{}\"\non_conflict = \"prompt\"\n",
        self.source.display(),
        self.target.display()
      ),
    )
    .unwrap();
  }

  /// Plant a real-file conflict and run `link <pkg>` on a PTY. `downs` is the
  /// number of Down arrows to send before Enter (1 = overwrite, 2 = skip in
  /// the four-item file-conflict select). Returns the exit code and the full
  /// PTY transcript (stdout + stderr) for assertions and diagnostics.
  fn drive(&self, pkg: &str, downs: usize) -> (u32, String) {
    let pkg_dir = self.source.join(pkg);
    fs::create_dir_all(&pkg_dir).unwrap();
    fs::write(pkg_dir.join(".zshrc"), "package contents\n").unwrap();
    fs::write(self.target.join(".zshrc"), "user contents\n").unwrap();
    self.run_pty(&["link", pkg], downs)
  }

  /// Spawn the real binary on a PTY, wait for the conflict select to render,
  /// send `downs` Down arrows + Enter, reap the child, and return its exit
  /// code plus the full transcript. A `ChildGuard` owns the child from the
  /// moment it is spawned so a panic cannot orphan it.
  fn run_pty(&self, args: &[&str], downs: usize) -> (u32, String) {
    let bin = assert_cmd::cargo::cargo_bin("rustow");
    let pair = native_pty_system()
      .openpty(PtySize {
        rows: 30,
        cols: 100,
        pixel_width: 0,
        pixel_height: 0,
      })
      .unwrap();
    let mut cmd = CommandBuilder::new(bin);
    cmd.args(args.iter().copied());
    cmd.env("HOME", &self.home);
    cmd.env("XDG_CONFIG_HOME", &self.xdg);
    cmd.env("XDG_CACHE_HOME", &self.xdg);
    cmd.env("DOTFILES_DIR", &self.source);
    cmd.env("DOTFILES_TARGET", &self.target);
    cmd.env("RUSTOW_NO_STOWRC", "1");
    cmd.env("TERM", "xterm-256color");
    cmd.env("NO_COLOR", "1");

    let child = pair.slave.spawn_command(cmd).unwrap();
    let mut guard = ChildGuard::new(child); // own it before anything else can panic
    drop(pair.slave); // parent must not hold the slave open: the reader needs EOF

    let output = Arc::new(Mutex::new(Vec::new()));
    let sink = Arc::clone(&output);
    let mut reader = pair.master.try_clone_reader().unwrap();
    let reader_thread = thread::spawn(move || {
      let mut buf = [0u8; 4096];
      loop {
        match reader.read(&mut buf) {
          Ok(0) => break,
          Ok(n) => sink.lock().unwrap().extend_from_slice(&buf[..n]),
          // EIO on Linux once the last slave fd is closed: normal end of output.
          Err(_) => break,
        }
      }
    });
    guard.attach_reader(reader_thread);

    let mut writer = pair.master.take_writer().unwrap();
    let prompt = b"what would you like to do?";
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
      if contains(&output.lock().unwrap(), prompt) {
        break;
      }
      if Instant::now() >= deadline {
        // Still send the keys: a child waiting for input must be released so
        // it exits cleanly. Assertions below will report the missing prompt.
        break;
      }
      thread::sleep(Duration::from_millis(10));
    }
    let mut keys = [0x1b, b'[', b'B'].repeat(downs); // Down arrow
    keys.push(b'\r'); // Enter
    let _ = writer.write_all(&keys); // EIO means the child already exited

    let code = guard.reap();
    guard.join_reader();
    let text = String::from_utf8_lossy(&output.lock().unwrap()).into_owned();
    (code, text)
  }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
  haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
fn pty_conflict_overwrite_replaces_file_with_link() {
  let r = rig();
  r.write_config();
  let (code, text) = r.drive("demo", 1);
  assert_eq!(code, 0, "transcript: {text}");
  assert!(contains(text.as_bytes(), b"what would you like to do?"));

  let dst = r.target.join(".zshrc");
  let meta = fs::symlink_metadata(&dst).unwrap();
  assert!(meta.file_type().is_symlink(), "not a symlink: {text}");
  assert_eq!(fs::read_to_string(&dst).unwrap(), "package contents\n");
}

#[test]
fn pty_conflict_skip_leaves_file_in_place() {
  let r = rig();
  r.write_config();
  let (code, text) = r.drive("demo", 2);
  assert_eq!(code, 0, "transcript: {text}");
  assert!(
    contains(text.as_bytes(), b"skipped 1"),
    "transcript: {text}"
  );

  let dst = r.target.join(".zshrc");
  let meta = fs::symlink_metadata(&dst).unwrap();
  assert!(meta.file_type().is_file(), "target was changed: {text}");
  assert_eq!(fs::read_to_string(&dst).unwrap(), "user contents\n");
}

/// Documents a known pre-existing gap (Task 2 reviewer note): with
/// `on_conflict = prompt` on a TTY, `relink` defers its conflict decision to
/// the post-unlink prompt, so aborting there reports "nothing was changed"
/// although the unlink already happened. Ignored because it asserts the
/// CORRECT post-fix behaviour, not today's defect: links stay in place and
/// the failure message is truthful. Run explicitly with `--ignored` to
/// reproduce the gap (it fails until the bug is fixed).
#[test]
#[ignore = "known bug: relink unlinks before the abort prompt; restore links before resolving"]
fn pty_relink_prompt_abort_keeps_links_and_reports_conflict() {
  let r = rig();
  r.write_config();
  for (pkg, files) in [
    ("alpha", [".bashrc", ".sharedrc"].as_slice()),
    ("beta", [".bashrc"].as_slice()),
  ] {
    let dir = r.source.join(pkg);
    fs::create_dir_all(&dir).unwrap();
    for f in files {
      fs::write(dir.join(f), format!("contents of {f}")).unwrap();
    }
  }
  // Link alpha non-interactively first (no conflicts in that plan).
  let setup = std::process::Command::new(assert_cmd::cargo::cargo_bin("rustow"))
    .args(["link", "alpha"])
    .env("HOME", &r.home)
    .env("XDG_CONFIG_HOME", &r.xdg)
    .env("XDG_CACHE_HOME", &r.xdg)
    .env("DOTFILES_DIR", &r.source)
    .env("DOTFILES_TARGET", &r.target)
    .env("RUSTOW_NO_STOWRC", "1")
    .output()
    .unwrap();
  assert!(setup.status.success());
  assert!(r.target.join(".bashrc").is_symlink());

  // At the conflict select: Down, Down -> "abort everything" (no regular file
  // at the destination here, so the items are overwrite/skip/abort).
  let (code, text) = r.run_pty(&["relink", "alpha", "beta"], 2);
  assert_eq!(code, 1, "transcript: {text}");
  // An accurate failure message names the conflict. Exact wording is
  // deliberately not pinned to today's misleading "nothing was changed"
  // line: after the fix that phrase may remain, but only when the tree
  // really is unchanged.
  assert!(contains(text.as_bytes(), b"conflict"), "transcript: {text}");
  // Correct behaviour: aborting at the post-unlink prompt leaves both links
  // where they were, pointing at alpha. Today they are already gone.
  for leaf in [".bashrc", ".sharedrc"] {
    let dst = r.target.join(leaf);
    let meta = fs::symlink_metadata(&dst)
      .unwrap_or_else(|e| panic!("{leaf} is gone ({e}); transcript: {text}"));
    assert!(meta.file_type().is_symlink(), "{leaf} is no longer a link");
    assert_eq!(
      fs::read_link(&dst).unwrap(),
      r.source.join("alpha").join(leaf),
      "{leaf} points somewhere else"
    );
  }
}
