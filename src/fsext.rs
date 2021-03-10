//! Filesystem manipulation operations.
//!
//! This module contains additional operations which are not present in
//! `std::fs` and `std::os::$platform`.

use anyhow::{anyhow, Result};
use nix::unistd;
use std::path::Path;

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
