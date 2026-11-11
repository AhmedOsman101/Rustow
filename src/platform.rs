//! Cross-platform symlink helper. On Windows, file and directory symlinks are
//! distinct; we pick based on what the original path points to. Windows
//! symlinks also need developer mode or admin privileges — failures surface
//! as ordinary IO errors there.

use std::path::Path;

use crate::error::{Error, Result};

#[cfg(unix)]
pub fn symlink(original: &Path, link: &Path) -> Result<()> {
  std::os::unix::fs::symlink(original, link).map_err(Error::io(link))
}

#[cfg(windows)]
pub fn symlink(original: &Path, link: &Path) -> Result<()> {
  let original = original.to_path_buf();
  let meta = original
    .symlink_metadata()
    .map_err(Error::io(Path::new(&original)))?;
  if meta.is_dir() {
    std::os::windows::fs::symlink_dir(&original, link).map_err(Error::io(link))
  } else {
    std::os::windows::fs::symlink_file(&original, link).map_err(Error::io(link))
  }
}
