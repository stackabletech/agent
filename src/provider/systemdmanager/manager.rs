//* A systemd unit manager that allows managing systemd services

use crate::provider::systemdmanager::service::SystemDUnit;
use dbus::blocking::SyncConnection;
use log::{debug, error, info};
use serde::de::Error;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::time::Duration;

enum UnitTypes {
    Service,
}

pub struct Manager {
    units_directory: PathBuf,
    user_mode: bool,
    systemd_connection: SystemDConnection,
}

impl Manager {
    pub fn new(units_directory: PathBuf, user_mode: bool) -> Self {
        Manager {
            units_directory,
            user_mode,
            systemd_connection: SystemDConnection::new(), // Unused at present
        }
    }

    // Write the unit file to disk and enable the service
    // TODO: should we maybe split enable out into a different function?
    pub fn load(&self, name: &str, unit: SystemDUnit) -> Result<(), anyhow::Error> {
        let target_file = self
            .units_directory
            .join(Manager::get_unit_file_path(name, UnitTypes::Service)?);
        debug!("Target file for service {} : {:?}", name, target_file);

        let mut unit_file = File::create(&target_file)?;
        unit_file.write(unit.get_unit_file_content().as_bytes());
        unit_file.flush();
        self.systemd_connection
            .enable_unit_file(&target_file.into_os_string().to_string_lossy());
        Ok(())
    }

    // Check if the unit name is valid and append .service if needed
    fn get_unit_file_path(name: &str, unit_type: UnitTypes) -> Result<String, anyhow::Error> {
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

    // Stop and disable the service, then delete the unit file from disk
    pub fn unload() -> Result<(), anyhow::Error> {
        Ok(())
    }

    pub fn reload() -> Result<(), anyhow::Error> {
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
    fn new() -> SystemDConnection {
        let connection = SyncConnection::new_system().expect("D-Bus connection failed");
        SystemDConnection {
            connection: connection,
            dest: "org.freedesktop.systemd1",
            node: "/org/freedesktop/systemd1",
            interface: "org.freedesktop.systemd1.Manager",
            timeout: Duration::from_millis(5000),
        }
    }

    /// Takes a unit name as input and attempts to start it.
    pub fn start_unit(&self, unit: &str) -> Result<u32, dbus::Error> {
        // create a wrapper struct around the connection that makes it easy
        // to send method calls to a specific destination and path.
        let proxy = self
            .connection
            .with_proxy(self.dest, self.node, self.timeout);
        proxy
            .method_call(self.interface, "StartUnit", (unit, "fail"))
            .and_then(|r: (u32,)| Ok(r.0))
    }

    /// Takes a unit name as input and attempts to stop it.
    pub fn stop_unit(&self, unit: &str) -> Result<u32, dbus::Error> {
        // create a wrapper struct around the connection that makes it easy
        // to send method calls to a specific destination and path.
        let proxy = self
            .connection
            .with_proxy(self.dest, self.node, self.timeout);
        proxy
            .method_call(self.interface, "StopUnit", (unit, "fail"))
            .and_then(|r: (u32,)| Ok(r.0))
    }

    /// THIS WORKS
    /// Takes the unit pathname of a service and enables it via dbus.
    /// If dbus replies with `[Bool(true), Array([], "(sss)")]`, the service is already enabled.
    pub fn enable_unit_file(&self, unit: &str) -> Option<String> {
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
                let (s) = reply;
                println!("{:?}", s);
                /* if format!("{:?}", reply.get_items()) == "[Bool(true), Array([], \"(sss)\")]" {
                    println!("{} already enabled", unit);
                } else {
                    println!("{} has been enabled", unit);
                }*/
                None
            }
            Err(reply) => {
                let error = format!("Error enabling {}:\n{:?}", unit, reply);
                println!("{}", error);
                Some(error)
            }
        }
    }
}

/*
   Disable(name string) error
   Load(name string, u unit.File) error
   Mask(name string) error
   Properties(name string) (map[string]interface{}, error)
   Property(name, property string) string
   Reload() error
   ServiceProperty(name, property string) string
   State(name string) (*unit.State, error)
   States(prefix string) (map[string]*unit.State, error)
   TriggerStart(name string) error
   TriggerStop(name string) error
   Unit(name string) string
   Units() ([]string, error)
   Unload(name string) error
*/
