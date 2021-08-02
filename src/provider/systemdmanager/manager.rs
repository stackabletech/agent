//! A module to allow managing systemd units - mostly services currently
//!
//! The module offers the ability to create, remove, start, stop, enable and
//! disable systemd units.
//!
use super::systemd1_api::{
    ActiveState, AsyncJobProxy, AsyncManagerProxy, AsyncServiceProxy, JobRemovedResult,
    JobRemovedSignal, ManagerSignals, StartMode, StopMode,
};
use crate::provider::systemdmanager::{systemd1_api::ServiceResult, systemdunit::SystemDUnit};
use crate::provider::StackableError;
use crate::provider::StackableError::RuntimeError;
use anyhow::anyhow;
use futures_util::{future, stream::StreamExt};
use log::debug;
use std::fs;
use std::fs::File;
use std::future::Future;
use std::io::Write;
use std::path::PathBuf;
use zbus::azync::Connection;

/// Enum that lists the supported unit types
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UnitTypes {
    Service,
}

/// The main way of interacting with this module, this struct offers
/// the public methods for managing service units.
///
/// Use [`SystemdManager::new`] to create a new instance.
pub struct SystemdManager {
    units_directory: PathBuf,
    proxy: AsyncManagerProxy<'static>,
    user_mode: bool, // TODO Use the same naming (user_mode or session_mode) everywhere
}

impl SystemdManager {
    /// Creates a new instance, takes a flag whether to run within the
    /// user session or manage services system-wide.
    pub async fn new(user_mode: bool, max_pods: u16) -> Result<Self, StackableError> {
        // Connect to session or system bus depending on the value of [user_mode]
        let connection = if user_mode {
            Connection::new_session().await.map_err(|e| RuntimeError {
                msg: format!(
                    "Could not create a connection to the systemd session bus: {}",
                    e
                ),
            })?
        } else {
            Connection::new_system().await.map_err(|e| RuntimeError {
                msg: format!(
                    "Could not create a connection to the systemd system-wide bus: {}",
                    e
                ),
            })?
        };

        // The maximum number of queued DBus messages must be higher
        // than the number of containers which can be started and
        // stopped simultaneously.
        let connection = connection.set_max_queued(max_pods as usize * 2);

        let proxy = AsyncManagerProxy::new(&connection).map_err(|e| RuntimeError {
            msg: format!(
                "Proxy for org.freedesktop.systemd1.Manager could not be created: {}",
                e
            ),
        })?;

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
            proxy,
            user_mode,
        })
    }

    pub fn is_user_mode(&self) -> bool {
        self.user_mode
    }

    // Internal helper method to remove an existing unit file or symlink
    fn delete_unit_file(&self, unit: &str) -> anyhow::Result<()> {
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
    /// The location of the unit file is determined by the value of `unit_file_path`:
    ///
    /// * None, the unit file will be created in the base directory that this manager was initialized
    /// with, which is either /lib/systemd/system or ~/.config/systemd/user depending on the value of
    /// `session`.
    /// * Some<PathBuf>, the unit file will be created at this location and linked into the proper
    /// systemd unit directory
    ///
    /// `force` determines if an existing unit file should be overwritten, if no  external unit file
    /// path is specified in `unit_file_path`. If this is false and the target file exists an error
    /// is returned.
    ///
    /// The value of `daemon_reload` controls whether a daemon reload is triggered after creating or
    /// linking the unit file.
    pub async fn create_unit(
        &self,
        unit: &SystemDUnit,
        unit_file_path: Option<PathBuf>,
        force: bool,
        daemon_reload: bool,
    ) -> anyhow::Result<()> {
        // Appends .service to name if necessary
        let linked_unit_file = unit_file_path.is_some();
        let unit_name = SystemdManager::get_unit_file_name(&unit.name, &unit.unit_type)?;

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

        // The following behavior distinguishes between a systemd unit that is defined in a file
        // external to the systemd units directory which is then symlinked to and a file that is
        // created directly in the systemd units dir.
        //
        // For the first case the _external_ file that will be symlinked to should have been written
        // or potentially overwritten above, which is why we bypass this entire conditional in that
        // case.
        // For the case where we need to symlink we check if a symlink already exists and if so
        // if force has been specified - only then do we remove an existing link before recreating
        // it.

        // Perform some pre-flight checks to ensure that writing the unit file doesn't clash
        // with any existing files
        if !linked_unit_file
            && target_file.exists()
            && fs::symlink_metadata(&target_file)?.file_type().is_symlink()
        {
            // Handle the special case where we need to replace a symlink with an actual file
            // This only occurs when switching from using a linked file to writing the file
            // directly into the units folder - should not happen in practice
            // In this case we need to remove the symlink
            fs::remove_file(&target_file)?;
        }

        let unit_file = self.units_directory.join(&unit_name);
        if unit_file.exists() && unit_file.symlink_metadata()?.file_type().is_file() {
            // Handle the special case where we need to replace an actual file with a symlink
            // This only occurs when switching from writing the file
            // directly into the units folder to using a linked file - should not happen in practice
            // In this case we need to remove the file
            fs::remove_file(&unit_file)?;
        }

        // We have handled the special case above, if the target file does not exist
        // at this point in time we write the file - doesn't matter if inside or outside
        // the systemd folder
        if !target_file.exists() {
            // Write unit file, no matter where
            // TODO: implement check for content equality
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
        }

        // If this is a linked unit file we need to call out to systemd to link this file
        if linked_unit_file {
            self.link_unit_file(&target_file.into_os_string().to_string_lossy(), force)
                .await?;
        }

        // Perform daemon reload if requested
        if daemon_reload {
            self.reload().await?;
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
    pub async fn remove_unit(&self, unit: &str, daemon_reload: bool) -> anyhow::Result<()> {
        debug!("Disabling unit [{}]", unit);
        if let Err(disable_error) = self.disable(unit).await {
            debug!(
                "Error disabling systemd unit [{}]: [{}]",
                unit, disable_error
            );
            return Err(disable_error);
        }

        // If we are not linking to the unit file but writing it directly in the
        // units folder it won't be removed by the dbus method call to `DisableUnitFiles`
        //from [disable], so we delete explicitly
        let unit_file = self.units_directory.join(&unit);
        if unit_file.exists() {
            debug!("Removing unit [{}] from systemd", unit);
            self.delete_unit_file(unit)?;
        }

        if daemon_reload {
            self.reload().await?;
        }
        Ok(())
    }

    /// Enables a systemd unit to be stared automatically at system boot - expects a fully named
    /// unit (which means: including the .service or other unit type).
    /// This either requires that the unit is known to systemd or an absolute path to a unit file
    /// to work.
    ///
    /// For a unit file to be _known_ it needs to either be located in the systemd unit folder, or
    /// linked into that folder - both actions can be performed by calling [`SystemdManager::create_unit`]
    pub async fn enable(&self, unit: &str) -> anyhow::Result<()> {
        // We don't do any checking around this and simply trust the user that either the name
        // of an existing and linked service was provided or this is an absolute path
        debug!("Trying to enable systemd unit [{}]", unit);

        match self.proxy.enable_unit_files(&[unit], false, true).await {
            Ok(_) => {
                debug!("Successfully enabled service [{}]", unit);
                Ok(())
            }
            Err(e) => Err(anyhow!("Error enabling service [{}]: {}", unit, e)),
        }
    }

    // Disable the systemd unit - which effectively means removing the symlink from the
    // multi-user.target subdirectory.
    pub async fn disable(&self, unit: &str) -> anyhow::Result<()> {
        debug!("Trying to disable systemd unit [{}]", unit);
        match self.proxy.disable_unit_files(&[unit], false).await {
            Ok(_) => {
                debug!("Successfully disabled service [{}]", unit);
                Ok(())
            }
            Err(e) => Err(anyhow!("Error disabling service [{}]: {}", unit, e)),
        }
    }

    /// Attempts to start a systemd unit
    /// [unit] is expected to be the name (including .<unittype>) of a service that is known to
    /// systemd at the time this is called.
    /// To make a service known please take a look at the [`SystemdManager::enable`] function.
    pub async fn start(&self, unit: &str) -> anyhow::Result<()> {
        debug!("Trying to start unit [{}]", unit);

        let result = self
            .call_method(|proxy| proxy.start_unit(unit, StartMode::Fail))
            .await;

        if result.is_ok() {
            debug!("Successfully started service [{}]", unit);
        }

        result.map_err(|e| anyhow!("Error starting service [{}]: {}", unit, e))
    }

    /// Attempts to stop a systemd unit
    /// [unit] is expected to be the name (including .<unittype>) of a service that is known to
    /// systemd at the time this is called.
    /// To make a service known please take a look at the [`SystemdManager::enable`] function.
    pub async fn stop(&self, unit: &str) -> anyhow::Result<()> {
        debug!("Trying to stop systemd unit [{}]", unit);

        let result = self
            .call_method(|proxy| proxy.stop_unit(unit, StopMode::Fail))
            .await;

        if result.is_ok() {
            debug!("Successfully stopped service [{}]", unit);
        }

        result.map_err(|e| anyhow!("Error stopping service [{}]: {}", unit, e))
    }

    /// Calls a systemd method and waits until the dependent job is
    /// finished.
    ///
    /// The given method enqueues a job in systemd and returns the job
    /// object. Systemd sends out a `JobRemoved` signal when the job is
    /// dequeued. The signal contains the reason for the dequeuing like
    /// `"done"`, `"failed"`, or `"canceled"`.
    ///
    /// This function subscribes to `JobRemoved` signals, calls the
    /// given method, awaits the signal for the corresponding job, and
    /// returns `Ok(())` if the result is [`JobRemovedResult::Done`].
    /// If the signal contains another result or no signal is returned
    /// (which should never happen) then an error with a corresponding
    /// message is returned.
    async fn call_method<'a, F, Fut>(&'a self, method: F) -> anyhow::Result<()>
    where
        F: Fn(&'a AsyncManagerProxy) -> Fut,
        Fut: Future<Output = zbus::Result<AsyncJobProxy<'a>>>,
    {
        let signals = self
            .proxy
            .receive_signal(ManagerSignals::JobRemoved.into())
            .await?
            .map(|message| message.body::<JobRemovedSignal>().unwrap());

        let job = method(&self.proxy).await?;

        let mut signals = signals
            .filter(|signal| future::ready(&signal.job.to_owned().into_inner() == job.path()));

        let signal = signals.next().await;

        match signal {
            Some(message) if message.result == JobRemovedResult::Done => Ok(()),
            Some(message) => Err(anyhow!("The systemd job failed: {:?}", message)),
            None => Err(anyhow!(
                "No signal was returned for the systemd job: {:?}",
                job
            )),
        }
    }

    // Perform a daemon-reload, this causes systemd to re-read all unit files on disk and
    // discover changes that have been performed since the last reload
    // This needs to be done after creating a new service unit before it can be targeted by
    // start / stop and similar commands.
    pub async fn reload(&self) -> anyhow::Result<()> {
        debug!("Performing daemon-reload..");

        match self.proxy.reload().await {
            Ok(_) => {
                debug!("Successfully performed daemon-reload");
                Ok(())
            }
            Err(e) => Err(anyhow!("Error performing daemon-reload: [{}]", e)),
        }
    }

    /// Checks if the ActiveState of the given unit is set to active.
    pub async fn is_running(&self, unit: &str) -> anyhow::Result<bool> {
        self.proxy
            .load_unit(unit)
            .await?
            .active_state()
            .await
            .map(|state| state == ActiveState::Active)
            .map_err(|e| anyhow!("Error receiving ActiveState of unit [{}]. {}", unit, e))
    }

    /// Checks if the result of the given service unit is not set to success.
    pub async fn failed(&self, unit: &str) -> anyhow::Result<bool> {
        let unit_proxy = self.proxy.load_unit(unit).await?;
        let service_proxy = AsyncServiceProxy::from(unit_proxy);
        service_proxy
            .result()
            .await
            .map(|state| state != ServiceResult::Success)
            .map_err(|e| anyhow!("Error receiving Result of unit [{}]. {}", unit, e))
    }

    /// Retrieves the invocation ID for the given unit.
    ///
    /// The invocation ID was introduced in systemd version 232.
    pub async fn get_invocation_id(&self, unit: &str) -> anyhow::Result<String> {
        self.proxy
            .load_unit(unit)
            .await?
            .invocation_id()
            .await
            .map(|invocation_id| invocation_id.to_string())
            .map_err(|e| anyhow!("Error receiving InvocationID of unit [{}]. {}", unit, e))
    }

    // Symlink a unit file into the systemd unit folder
    // This is not public on purpose, as [create] should be the normal way to link unit files
    // when using this crate
    async fn link_unit_file(&self, unit: &str, force: bool) -> anyhow::Result<()> {
        debug!("Linking [{}]", unit);
        self.proxy.link_unit_files(&[unit], false, force).await?;
        Ok(())
    }

    // Check if the unit name is valid and append .service if needed
    // Cannot currently fail, I'll need to dig into what is a valid unit
    // name before adding checks
    #[allow(clippy::unnecessary_wraps)]
    fn get_unit_file_name(name: &str, unit_type: &UnitTypes) -> anyhow::Result<String> {
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
