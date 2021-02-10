//! A manager that allows managing systemd units

use crate::provider::systemdmanager::service::SystemDUnit;
use anyhow::anyhow;
use dbus::arg::{AppendAll, ReadAll};
use dbus::blocking::SyncConnection;
use dbus::strings::Member;
use dbus::Path;
use log::debug;
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Debug)]
pub enum UnitTypes {
    Service,
}

pub const SYSTEMD_DESTINATION: &str = "org.freedesktop.systemd1";
pub const SYSTEMD_NODE: &str = "/org/freedesktop/systemd1";
pub const SYSTEMD_MANAGER_INTERFACE: &str = "org.freedesktop.systemd1.Manager";

pub struct SystemdManager {
    units_directory: PathBuf,
    connection: SyncConnection, //TODO does this need to be closed?
    timeout: Duration,
}

impl Default for SystemdManager {
    fn default() -> Self {
        // If this panics we broke something in the code, as this is all constant values that
        // should work
        SystemdManager::new(false, Duration::from_secs(5)).unwrap()
    }
}

impl SystemdManager {
    pub fn new(user_mode: bool, timeout: Duration) -> Result<Self, anyhow::Error> {
        // Connect to session or system bus depending on the value of [user_mode]
        let connection = if user_mode {
            SyncConnection::new_session()?
        } else {
            SyncConnection::new_system()?
        };

        // Depending on whether we are supposed to run in user space or system-wide
        // we'll pick the default directory to initialize the systemd manager with
        // This allows creating unit files either directly in the systemd folder by
        // passing in just a filename, or symlink them by passing in an absolute
        // path
        let units_directory = if user_mode {
            PathBuf::from(shellexpand::tilde("~/.config/systemd/user").to_string())
        } else {
            PathBuf::from("/lib/systemd/system")
        };

        Ok(SystemdManager {
            units_directory,
            connection,
            timeout,
        })
    }

    // The main method for interacting with dbus, all other functions will delegate the actual
    // dbus access to this function.
    // Private on purpose as this should not be used by external dependencies
    fn method_call<'m, R: ReadAll, A: AppendAll, M: Into<Member<'m>>>(
        &self,
        m: M,
        args: A,
    ) -> Result<R, dbus::Error> {
        let proxy = self
            .connection
            .with_proxy(SYSTEMD_DESTINATION, SYSTEMD_NODE, self.timeout);
        proxy.method_call(SYSTEMD_MANAGER_INTERFACE, m, args)
    }

    // Internal helper method to remove an existing unit file or symlink
    fn delete_unit_file(&self, unit: &str) -> Result<(), anyhow::Error> {
        let unit_file = self.units_directory.clone().join(&unit);
        debug!("Removing [{:?}]", unit_file);

        match fs::remove_file(&unit_file) {
            Ok(()) => Ok(()),
            Err(delete_error) => {
                debug!(
                    "Failed to remove existing unit file [{:?}] for systemd unit [{}]",
                    unit_file, unit
                );
                Err(anyhow::Error::from(delete_error))
            }
        }
    }

    /// Write the proper unit file for [unit] to disk.
    /// The location of the unit file is determined by the value of [unit_file_path]:
    ///
    /// * None, the unit file will be created in the base directory that this manager was initialized
    /// with, which is either /lib/systemd/system or ~/.config/systemd/user depending on the value of
    /// [session].
    /// * Some<PathBuf>, the unit file will be created at this location and linked into the proper
    /// systemd unit directory TODO: this is not implemented yet
    ///
    /// [force] determines if an existing unit file should be overwritten, if no  external unit file
    /// path is specified in [unit_file_path]. If this is false and the target file exists an error
    /// is returned.
    ///
    /// The value of [daemon_reload] controls whether a daemon reload is triggered after creating or
    /// linking the unit file.
    pub fn create_unit(
        &self,
        unit: &SystemDUnit,
        unit_file_path: Option<PathBuf>,
        force: bool,
        daemon_reload: bool,
    ) -> Result<(), anyhow::Error> {
        // Appends .service to name if necessary
        let linked_unit_file = unit_file_path.is_some();
        let unit_name = SystemdManager::get_unit_file_name(unit.name.as_ref(), &unit.unit_type)?;

        // Check if a path was provided for the unit file, otherwise use the base directory
        let target_file = if let Some(path) = unit_file_path {
            path.join(&unit_name)
        } else {
            // TODO: I think we can get away with a reference here, but not sure yet,
            //  that would mean looking into get_unit_file_name returning a &str, _I think_
            self.units_directory.clone().join(&unit_name)
        };

        debug!(
            "Target file for service [{}] : [{:?}]",
            &unit_name, &target_file
        );

        // If no external file was created check if the file currently exists
        if !linked_unit_file {
            debug!("Target file [{:?}] already exists", &target_file);
            if target_file.exists() {
                // Target file already exists, remove if force is supplied, otherwise abort
                if force {
                    self.delete_unit_file(&unit_name)?;
                } else {
                    return Err(anyhow!(
                        "Target unit file [{:?}] exists and force parameter was not specified.",
                        target_file
                    ));
                }
            }
        }

        // Write unit file
        let mut unit_file = match File::create(&target_file) {
            Ok(file) => file,
            Err(e) => {
                debug!(
                    "Error occurred when creating unit file [{}]: [{}]",
                    unit_name, e
                );
                return Err(anyhow::Error::from(e));
            }
        };
        unit_file.write_all(unit.get_unit_file_content().as_bytes())?;
        unit_file.flush()?;

        // If this is a linked unit file we need to call out to systemd to link this file
        if linked_unit_file {
            self.link_unit_file(&target_file.into_os_string().to_string_lossy())?;
        }

        // Perform daemon reload if requested
        if daemon_reload {
            self.reload()?;
        }
        Ok(())
    }

    /// Removes a unit from systemd.
    /// Depending on what is passed in the [unit] parameter this means one of two things:
    ///
    /// * if an absolute file path is passed, the symlink to this file is deleted from the
    /// systemd unit folder
    /// * if a unit name is passed an attempt is made to unlink the unit via a dbus call
    ///
    /// Calling this function means an implicit disabling of the service, if it was enabled.
    ///
    pub fn remove_unit(&self, unit: &str, daemon_reload: bool) -> Result<(), anyhow::Error> {
        debug!("Disabling unit [{}]", unit);
        if let Err(disable_error) = self.disable(unit) {
            debug!(
                "Error disabling systemd unit [{}]: [{}]",
                unit, disable_error
            );
            return Err(disable_error);
        }

        debug!("Removing unit [{}] from systemd", unit);
        self.delete_unit_file(&unit)?;

        if daemon_reload {
            self.reload()?;
        }
        Ok(())
    }

    /// Enables a systemd unit to be stared automatically at system boot - expects a fully named
    /// unit (which means: including the .service or other unit type).
    /// This either requires that the unit is known to systemd or an absolute path to a unit file
    /// to work.
    ///
    /// For a unit file to be _known_ it needs to either be located in the systemd unit folder, or
    /// linked into that folder - both actions can be performed by calling [create_unit]
    pub fn enable(&self, unit: &str) -> Result<(), anyhow::Error> {
        // We don't do any checking around this and simply trust the user that either the name
        // of an existing and linked service was provided or this is an absolute path
        debug!("Trying to enable systemd unit [{}]", unit);

        match self
            .method_call("EnableUnitFiles", (&[unit][..], false, true))
            .map(|_: ()| ())
        {
            Ok(()) => {
                debug!("Successfully started service [{}]", unit);
                Ok(())
            }
            Err(e) => {
                debug!("Error: [{}]", e);
                Err(anyhow!("Error starting service [{}]: {}", unit, e))
            }
        }
    }

    // Stop and disable the service, then delete the unit file from disk
    pub fn disable(&self, unit: &str) -> Result<Vec<(String, String, String)>, anyhow::Error> {
        debug!("Trying to disable systemd unit [{}]", unit);
        match self
            .method_call("DisableUnitFiles", (&[unit][..], false))
            .map(|r: (Vec<(String, String, String)>,)| r.0)
        {
            Ok(result) => {
                debug!("Successfully disabled service [{}]", unit);
                println!("{:?}", result);
                Ok(result)
            }
            Err(e) => {
                debug!("Error: [{}]", e);
                Err(anyhow!("Error starting service [{}]: {}", unit, e))
            }
        }
    }

    /// Attempts to start a systemd unit
    /// [unit] is expected to be the name (including .<unittype>) of a service that is known to
    /// systemd at the time this is called.
    /// To make a service known please take a look at the [enable] function.
    pub fn start(&self, unit: &str) -> Result<(), anyhow::Error> {
        debug!("Attempting to start unit {}", unit);

        match self
            .method_call("StartUnit", (unit, "fail"))
            .map(|r: (Path,)| r.0)
        {
            Ok(result) => {
                debug!("Successfully started service [{}]: [{}]", unit, result);
                Ok(())
            }
            Err(e) => {
                debug!("Error: [{}]", e);
                Err(anyhow!("Error starting service [{}]: {}", unit, e))
            }
        }
    }

    /// Attempts to stop a systemd unit
    /// [unit] is expected to be the name (including .<unittype>) of a service that is known to
    /// systemd at the time this is called.
    /// To make a service known please take a look at the [enable] function.
    pub fn stop(&self, unit: &str) -> Result<(), anyhow::Error> {
        debug!("Trying to stop systemd unit [{}]", unit);

        match self
            .method_call("StopUnit", (unit, "fail"))
            .map(|r: (Path,)| r.0)
        {
            Ok(result) => {
                debug!("Successfully stopped service [{}]: [{}]", unit, result);
                Ok(())
            }
            Err(e) => {
                debug!("Error: [{}]", e);
                Err(anyhow!("Error s service [{}]: {}", unit, e))
            }
        }
    }

    // Perform a daemon-reload, this causes systemd to re-read all unit files on disk and
    // discover changes that have been performed since the last reload
    // This needs to be done after creating a new service unit before it can be targeted by
    // start / stop and similar commands.
    pub fn reload(&self) -> Result<(), anyhow::Error> {
        debug!("Performing daemon-reload..");

        match self.method_call("Reload", ()).map(|_: ()| ()) {
            Ok(_) => {
                debug!("Successfully performed daemon-reload");
                Ok(())
            }
            Err(e) => {
                debug!("Error: [{}]", e);
                Err(anyhow!("Error performing daemon-reload: [{}]", e))
            }
        }
    }

    // Symlink a unit file into the systemd unit folder
    // This is not public on purpose, as [create] should be the normal way to link unit files
    // when using this crate
    fn link_unit_file(&self, unit: &str) -> Result<(), dbus::Error> {
        debug!("Linking [{}]", unit);
        self.method_call("LinkUnitFiles", (&[unit][..], false, true))
            .map(|_: ()| ())
    }

    // Check if the unit name is valid and append .service if needed
    // Cannot currently fail, I'll need to dig into what is a valid unit
    // name before adding checks
    #[allow(clippy::unnecessary_wraps)]
    fn get_unit_file_name(name: &str, unit_type: &UnitTypes) -> Result<String, anyhow::Error> {
        // TODO: what are valid systemd unit names?

        // Append proper extension for unit type to file name
        let extension = match unit_type {
            UnitTypes::Service => ".service",
        };

        let mut result = String::from(name);
        if !name.ends_with(extension) {
            result.push_str(extension);
        }
        Ok(result)
    }
}
