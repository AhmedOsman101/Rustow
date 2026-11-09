use std::path::PathBuf;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
  #[error("I/O error at {path}: {source}")]
  Io {
    path: PathBuf,
    #[source]
    source: std::io::Error,
  },

  #[error("config error: {0}")]
  Config(String),

  #[error("source not set — run `rustow config init` or set DOTFILES_DIR")]
  SourceNotSet,

  #[error("conflicts detected:\n{0}")]
  Conflicts(String),

  #[error("package not found: {0}")]
  PackageNotFound(String),

  #[error("{0}")]
  Message(String),

  #[error("aborted by user")]
  Aborted,
}

impl Error {
  pub fn io(path: impl Into<PathBuf>) -> impl FnOnce(std::io::Error) -> Error {
    let path = path.into();
    move |source| Error::Io { path, source }
  }
}
