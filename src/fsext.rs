//! Filesystem manipulation operations.
//!
//! This module contains additional operations which are not present in
//! `std::fs` and `std::os::$platform`.

use std::io;
use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, Result};
use nix::{libc::O_TMPFILE, unistd};
use tokio::fs::OpenOptions;

/// User identifier
pub struct Uid(unistd::Uid);

impl Uid {
    /// Gets a Uid by user name.
    ///
    /// If no user with the given `user_name` exists then `Ok(None)` is returned.
    ///
    /// # Errors
    ///
    /// If this function encounters any form of I/O or other error, an error
    /// variant will be returned.
    pub fn from_name(user_name: &str) -> Result<Option<Uid>> {
        match unistd::User::from_name(user_name) {
            Ok(maybe_user) => Ok(maybe_user.map(|user| Uid(user.uid))),
            Err(err) => Err(anyhow!("Could not retrieve user [{}]. {}", user_name, err)),
        }
    }
}

/// Changes the ownership of the file or directory at `path` to be owned by the
/// given `uid`.
///
/// # Errors
///
/// If this function encounters any form of I/O or other error, an error
/// variant will be returned.
pub fn change_owner(path: &Path, uid: &Uid) -> Result<()> {
    Ok(unistd::chown(path, Some(uid.0), None)?)
}

/// Changes the ownership of the file or directory at `path` recursively to be
/// owned by the given `uid`.
///
/// # Errors
///
/// If this function encounters any form of I/O or other error, an error
/// variant will be returned.
pub fn change_owner_recursively(root_path: &Path, uid: &Uid) -> Result<()> {
    visit_recursively(root_path, &|path| change_owner(path, uid))
}

/// Calls the function `cb` on the given `path` and its contents recursively.
fn visit_recursively<F>(path: &Path, cb: &F) -> Result<()>
where
    F: Fn(&Path) -> Result<()>,
{
    cb(path)?;
    if path.is_dir() {
        for entry in path.read_dir()? {
            visit_recursively(entry?.path().as_path(), cb)?;
        }
    }
    Ok(())
}

/// Checks if the given directory exists and is writable by the current
/// process.
///
/// The check is performed by creating an unnamed temporary file in the
/// given directory. The file will be automatically removed by the
/// operating system when the last handle is closed.
pub async fn check_dir_is_writable(directory: &Path) -> io::Result<()> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(O_TMPFILE)
        .open(directory)
        .await
        .map(|_| ())
}

/// Normalizes a path.
///
/// In contrast to [`std::fs::canonicalize`] the path does not need to
/// exist.
///
/// # Examples
///
/// ```rust
/// # use stackable_agent::fsext::*;
/// use std::path::Path;
///
/// assert_eq!(Path::new("foo/bar"), normalize_path(Path::new("foo//bar")));
/// assert_eq!(Path::new("foo/bar"), normalize_path(Path::new("foo/./bar")));
/// assert_eq!(Path::new("foo/bar"), normalize_path(Path::new("foo/bar/.")));
/// assert_eq!(Path::new("foo/../bar"), normalize_path(Path::new("foo/../bar")));
/// assert_eq!(Path::new("foo/bar/.."), normalize_path(Path::new("foo/bar/..")));
/// assert_eq!(Path::new("/foo"), normalize_path(Path::new("/foo")));
/// assert_eq!(Path::new("./foo"), normalize_path(Path::new("./foo")));
/// assert_eq!(Path::new("foo"), normalize_path(Path::new("foo/")));
/// assert_eq!(Path::new("foo"), normalize_path(Path::new("foo")));
/// assert_eq!(Path::new("/"), normalize_path(Path::new("/")));
/// assert_eq!(Path::new("."), normalize_path(Path::new(".")));
/// assert_eq!(Path::new(".."), normalize_path(Path::new("..")));
/// assert_eq!(Path::new(""), normalize_path(Path::new("")));
/// ```
pub fn normalize_path(path: &Path) -> PathBuf {
    path.components().collect()
}

/// Returns true if the given path could reference a file.
///
/// In contrast to [`std::path::Path::is_file`] the file does not need
/// to exist.
///
/// Use normalized paths to avoid confusing results.
///
/// # Examples
///
/// ```rust
/// # use stackable_agent::fsext::*;
/// use std::path::Path;
///
/// assert!(is_valid_file_path(Path::new("foo/bar")));
/// assert!(is_valid_file_path(Path::new("foo/bar/")));
/// assert!(is_valid_file_path(Path::new("foo/bar/.")));
///
/// assert!(!is_valid_file_path(Path::new("foo/bar/..")));
/// assert!(!is_valid_file_path(Path::new("/")));
/// assert!(!is_valid_file_path(Path::new(".")));
/// assert!(!is_valid_file_path(Path::new("..")));
/// assert!(!is_valid_file_path(Path::new("")));
/// ```
pub fn is_valid_file_path(path: &Path) -> bool {
    matches!(path.components().last(), Some(Component::Normal(_)))
}
