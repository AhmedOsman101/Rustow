//! Interactive conflict resolution driven through a real pseudo-terminal.
//!
//! `dialoguer::Select` only prompts when stdin is a TTY, so a piped-stdin
//! integration test cannot reach the overwrite/skip choices at all. These
//! tests spawn the real binary attached to a PTY, wait for the conflict select
//! to render, then send arrow keys + Enter to pick an option. Every fixture
//! isolates HOME / XDG_* / DOTFILES_* under `/tmp/opencode`; the real dotfiles
//! are never touched. The child is always driven to completion and reaped
//! here — no signals, no stray processes.

use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

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
  /// code plus the full transcript. The child is always released and waited
  /// on — this test never signals or reparents a process.
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

    let mut child = pair.slave.spawn_command(cmd).unwrap();
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

    let status = child.wait().unwrap();
    reader_thread.join().unwrap();
    let text = String::from_utf8_lossy(&output.lock().unwrap()).into_owned();
    (status.exit_code(), text)
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

/// Pins a known pre-existing gap (Task 2 reviewer note): with
/// `on_conflict = prompt` on a TTY, `relink` defers its conflict decision to
/// the post-unlink prompt. Choosing "abort everything" there then reports
/// "nothing was changed" even though the unlink already happened. The test
/// records today's behaviour so a future fix cannot change it silently.
#[test]
fn pty_relink_prompt_abort_reports_nothing_changed_after_unlink() {
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
  assert!(
    contains(text.as_bytes(), b"nothing was changed"),
    "transcript: {text}"
  );
  // The gap: the unlink phase already removed both links.
  assert!(!r.target.join(".bashrc").exists(), "transcript: {text}");
  assert!(!r.target.join(".sharedrc").exists(), "transcript: {text}");
}
