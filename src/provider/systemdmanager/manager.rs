//* A systemd unit manager that allows managing systemd services

use crate::provider::systemdmanager::service::SystemDUnit;
use anyhow::anyhow;
use dbus::blocking::SyncConnection;
use dbus::Path;
use log::{debug, error, info};
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

pub enum UnitTypes {
    Service,
}

pub struct Manager {
    units_directory: PathBuf,
    systemd_connection: SystemDConnection,
}

impl Manager {
    pub fn new(units_directory: PathBuf, user_mode: bool) -> Self {
        Manager {
            units_directory,
            systemd_connection: SystemDConnection::new(user_mode),
        }
    }

    // Write the unit file to disk and enable the service
    // TODO: should we maybe split enable out into a different function?
    pub fn load(
        &self,
        name: &str,
        unit: &SystemDUnit,
        unit_type: UnitTypes,
    ) -> Result<(), anyhow::Error> {
        // Appends .service to name if necessary
        let unit_name = Manager::get_unit_file_name(name, unit_type)?;

        let unit_path = PathBuf::from(&unit_name);

        // Check if an absolute path was provided
        // if this is an absolute path the file will be written there, otherwise
        // the file will be created in the unit directory
        let target_file = if unit_path.is_absolute() {
            unit_path
        } else {
            self.units_directory.join(unit_path)
        };

        //let target_file = self.units_directory.join(&unit_name);
        debug!("Target file for service {} : {:?}", name, target_file);

        let mut unit_file = match File::create(&target_file) {
            Ok(file) => file,
            Err(e) => {
                error!("Error ocurred when creating unit file [{}]: [{}]", name, e);
                return Err(anyhow::Error::from(e));
            }
        };

        unit_file.write_all(unit.get_unit_file_content().as_bytes())?;
        unit_file.flush()?;

        self.systemd_connection
            .enable_unit_file(&target_file.into_os_string().to_string_lossy())?;
        self.reload()?;

        Ok(())
    }

    // Stop and disable the service, then delete the unit file from disk
    pub fn unload(&self, name: &str, unit_type: UnitTypes) -> Result<(), anyhow::Error> {
        // Appends .service to name if necessary
        let unit_name = Manager::get_unit_file_name(name, unit_type)?;

        self.systemd_connection
            .disable_unit_file(&unit_name)
            .map_err(anyhow::Error::from)
    }

    pub fn start(&self, name: &str) -> Result<(), anyhow::Error> {
        let unit_name = Manager::get_unit_file_name(name, UnitTypes::Service)?;
        match self.systemd_connection.start_unit(&unit_name) {
            Ok(result) => info!("Successfully started service [{}]: [{}]", unit_name, result),
            Err(e) => {
                error!("Error: [{}]", e);
                return Err(anyhow!("Error starting service [{}]: {}", name, e));
            }
        };
        Ok(())
    }

    pub fn stop(&self, name: &str) -> Result<(), anyhow::Error> {
        let unit_name = Manager::get_unit_file_name(name, UnitTypes::Service)?;
        match self.systemd_connection.stop_unit(&unit_name) {
            Ok(result) => info!("Successfully stopped service [{}]: [{}]", unit_name, result),
            Err(e) => {
                error!("Error: [{}]", e);
                return Err(anyhow!("Error stopping service [{}]: {}", name, e));
            }
        };
        Ok(())
    }

    /// TODO: Make this actually do something, currently it only returns true
    #[allow(unused_variables)]
    pub fn is_active(&self, name: &str) -> Result<bool, anyhow::Error> {
        /*let unit_name = Manager::get_unit_file_name(name, UnitTypes::Service)?;
        match self.systemd_connection.get_unit_status(&unit_name) {
            Ok(result) => debug!(
                "Successfully retrieved state for unit [{}]: [{}]",
                unit_name, result
            ),
            Err(e) => {
                error!("Error: [{}]", e);
                return Err(anyhow!("Error getting unit state [{}]: {}", name, e));
            }
        };*/
        Ok(true)
    }

    // Check if the unit name is valid and append .service if needed
    // Cannot currently fail, I'll need to dig into what is a valid unit
    // name before adding checks
    #[allow(clippy::unnecessary_wraps)]
    fn get_unit_file_name(name: &str, unit_type: UnitTypes) -> Result<String, anyhow::Error> {
        // TODO: what are valid systemd unit names?

        // Append proper extension for unit type to file name
        let extension = match unit_type {
            UnitTypes::Service => ".service",
        };

        // Todo: fugly
        if !name.ends_with(extension) {
            let mut result = String::from(name);
            result.push_str(extension);
            Ok(result)
        } else {
            Ok(String::from(name))
        }
    }

    pub fn reload(&self) -> Result<(), anyhow::Error> {
        match self.systemd_connection.reload() {
            Ok(_) => info!("Successfully performed daemon-reload"),
            Err(e) => {
                error!("Error: [{}]", e);
                return Err(anyhow!("Error performing daemon-reload: [{}]", e));
            }
        };
        Ok(())
    }
}

struct SystemDConnection {
    connection: SyncConnection, //TODO does this need to be closed?
    dest: &'static str,
    node: &'static str,
    interface: &'static str,
    timeout: Duration,
}

impl SystemDConnection {
    fn new(user_mode: bool) -> SystemDConnection {
        let connection = if user_mode {
            SyncConnection::new_session().expect("Session D-Bus connection failed")
        } else {
            SyncConnection::new_system().expect("System D-Bus connection failed")
        };

        SystemDConnection {
            connection,
            dest: "org.freedesktop.systemd1",
            node: "/org/freedesktop/systemd1",
            interface: "org.freedesktop.systemd1.Manager",
            timeout: Duration::from_millis(5000),
        }
    }

    pub fn reload(&self) -> Result<(), dbus::Error> {
        let proxy = self
            .connection
            .with_proxy(self.dest, self.node, self.timeout);
        proxy
            .method_call(self.interface, "Reload", ())
            .map(|_: ()| ())
    }

    /// Takes a unit name as input and attempts to start it.
    pub fn start_unit(&self, unit: &str) -> Result<Path, dbus::Error> {
        // create a wrapper struct around the connection that makes it easy
        // to send method calls to a specific destination and path.
        info!("Attempting to start unit {}", unit);
        let proxy = self
            .connection
            .with_proxy(self.dest, self.node, self.timeout);
        proxy
            .method_call(self.interface, "StartUnit", (unit, "fail"))
            .map(|r: (Path,)| r.0)
    }

    /// Takes a unit name as input and attempts to stop it.
    pub fn stop_unit(&self, unit: &str) -> Result<Path, dbus::Error> {
        // create a wrapper struct around the connection that makes it easy
        // to send method calls to a specific destination and path.
        info!("Attempting to stop unit {}", unit);
        let proxy = self
            .connection
            .with_proxy(self.dest, self.node, self.timeout);
        proxy
            .method_call(self.interface, "StopUnit", (unit, "fail"))
            .map(|r: (Path,)| r.0)
    }

    /// Takes the unit pathname of a service and enables it via dbus.
    /// If dbus replies with `[Bool(true), Array([], "(sss)")]`, the service is already enabled.
    pub fn enable_unit_file(&self, unit: &str) -> Result<(), dbus::Error> {
        debug!("Enabling unit from file [{}]", unit);
        let proxy = self
            .connection
            .with_proxy(self.dest, self.node, self.timeout);
        let runtime = false;
        let force = true;
        let result: Result<(), dbus::Error> = proxy.method_call(
            self.interface,
            "EnableUnitFiles",
            (&[unit][..], runtime, force),
        );
        match result {
            Ok(reply) => {
                let s = reply;
                info!("Successfully loaded unit [{}] with result [{:?}]", unit, s);
                Ok(())
            }
            Err(e) => {
                error!("Error enabling unit [{}]", unit);
                Err(e)
            }
        }
    }

    // TODO: this doesn't work yet, the symlink is not deleted for some reason
    // I'll need to investigate this
    pub fn disable_unit_file(&self, unit: &str) -> Result<(), dbus::Error> {
        let proxy = self
            .connection
            .with_proxy(self.dest, self.node, self.timeout);
        let force = true;
        let result: Result<(Vec<String>,), dbus::Error> =
            proxy.method_call(self.interface, "DisableUnitFiles", (&[unit][..], force));

        match &result {
            Ok(_) => info!("Successfully disabled service!"),
            Err(e) => error!("Error disabling service: [{}]", e),
        }

        result.map(|_| ())
    }
}
