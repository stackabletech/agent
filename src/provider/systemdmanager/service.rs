//! Exposes methods from the systemd unit and service interfaces.
use super::systemd1_api::{
    ActiveState, AsyncManagerProxy, AsyncServiceProxy, AsyncUnitProxy, SUB_STATE_SERVICE_EXITED,
};
use anyhow::anyhow;

/// Represents the state of a service unit object.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ServiceState {
    /// The service was not started yet.
    Created,
    /// The service was started and is currently running or restarting.
    Started,
    /// The service terminated successfully and will not be restarted.
    Succeeded,
    /// The service terminated unsuccessfully and will not be restarted.
    Failed,
}

/// Stores proxies of a systemd unit and service
#[derive(Clone, Debug)]
pub struct SystemdService {
    file: String,
    unit_proxy: AsyncUnitProxy<'static>,
    service_proxy: AsyncServiceProxy<'static>,
}

impl SystemdService {
    pub async fn new(
        file: &str,
        manager_proxy: &AsyncManagerProxy<'static>,
    ) -> anyhow::Result<Self> {
        let unit_object_path = manager_proxy.load_unit(file).await?;

        // Caching of properties is disabled until it is more performant
        // (see https://gitlab.freedesktop.org/dbus/zbus/-/issues/184)

        let unit_proxy = AsyncUnitProxy::builder(manager_proxy.connection())
            .cache_properties(false)
            .path(unit_object_path)
            .unwrap() // safe because load_unit always returns a valid path
            .build()
            .await
            .unwrap(); // safe because destination, path, and interface are set

        let service_proxy = AsyncServiceProxy::builder(unit_proxy.connection())
            .cache_properties(false)
            .path(unit_proxy.path().to_owned())
            .unwrap() // safe because the path is taken from an existing proxy
            .build()
            .await
            .unwrap(); // safe because destination, path, and interface are set

        Ok(SystemdService {
            file: file.into(),
            unit_proxy,
            service_proxy,
        })
    }

    /// Returns the filename of the systemd unit.
    pub fn file(&self) -> String {
        self.file.clone()
    }

    /// Returns a coarse-grained state of the service unit object.
    ///
    /// It is assumed that RemainAfterExit is set to "yes" in the given
    /// unit if the service can terminate. Otherwise it would not be
    /// possible to distinguish between "inactive and never run" and
    /// "inactive and terminated successfully".
    pub async fn service_state(&self) -> anyhow::Result<ServiceState> {
        let active_state = self.unit_proxy.active_state().await?;

        let service_state = match active_state {
            ActiveState::Inactive => {
                // ActiveState "inactive" means in general that the
                // previous run was successful or no previous run has
                // taken place yet. If RemainAfterExit is set to "yes"
                // then a successfully terminated service stays in
                // ActiveState "active" and only a service which was not
                // started before is in ActiveState "inactive". It is
                // assumed here that RemainAfterExit is enabled.
                ServiceState::Created
            }
            ActiveState::Active => {
                let sub_state = self.unit_proxy.sub_state().await?;
                if sub_state == SUB_STATE_SERVICE_EXITED {
                    // The service terminated successfully (otherwise
                    // ActiveState would be set to "failed") and will
                    // not be restarted (otherwise ActiveState would be
                    // set to "activating") and RemainAfterExit is set
                    // to "yes" (otherwise ActiveState would be set to
                    // "inactive"). It is assumed here that
                    // RemainAfterExit is enabled.
                    ServiceState::Succeeded
                } else {
                    ServiceState::Started
                }
            }
            ActiveState::Failed => {
                // The service terminated unsuccessfully and will not be
                // restarted (otherwise ActiveState would be set to
                // "activating").
                ServiceState::Failed
            }
            ActiveState::Reloading => ServiceState::Started,
            ActiveState::Activating => ServiceState::Started,
            ActiveState::Deactivating => ServiceState::Started,
        };

        Ok(service_state)
    }

    /// Retrieves the current restart count.
    ///
    /// The restart counter was introduced in systemd version 235.
    pub async fn restart_count(&self) -> anyhow::Result<u32> {
        self.service_proxy
            .nrestarts()
            .await
            .map_err(|e| anyhow!("Error receiving NRestarts of unit [{}]. {}", self.file, e))
    }

    /// Retrieves the current invocation ID.
    ///
    /// The invocation ID was introduced in systemd version 232.
    pub async fn invocation_id(&self) -> anyhow::Result<String> {
        self.unit_proxy
            .invocation_id()
            .await
            .map(|invocation_id| invocation_id.to_string())
            .map_err(|error| {
                anyhow!(
                    "InvocationID of systemd unit [{}] cannot be retrieved: {}",
                    self.file,
                    error
                )
            })
    }
}
