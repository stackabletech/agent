use anyhow::{anyhow, Result};
use nix::unistd;
use std::path::Path;

pub struct Uid(unistd::Uid);

impl Uid {
    pub fn from_name(user_name: &str) -> Result<Option<Uid>> {
        match unistd::User::from_name(user_name) {
            Ok(maybe_user) => Ok(maybe_user.map(|user| Uid(user.uid))),
            Err(err) => Err(anyhow!("Could not retrieve user [{}]. {}", user_name, err)),
        }
    }
}

pub fn change_owner(path: &Path, uid: &Uid) -> Result<()> {
    Ok(unistd::chown(path, Some(uid.0), None)?)
}

pub fn change_owner_recursively(root_path: &Path, uid: &Uid) -> Result<()> {
    visit_recursively(root_path, &|path| change_owner(path, uid))
}

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
