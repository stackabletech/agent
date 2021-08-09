//! Exposes methods from the systemd unit and service interfaces.
use super::systemd1_api::{ActiveState, AsyncManagerProxy, AsyncServiceProxy, AsyncUnitProxy};
use crate::provider::systemdmanager::systemd1_api::ServiceResult;
use anyhow::anyhow;

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

    /// Checks if the ActiveState is set to active.
    pub async fn is_running(&self) -> anyhow::Result<bool> {
        self.unit_proxy
            .active_state()
            .await
            .map(|state| state == ActiveState::Active)
            .map_err(|error| {
                anyhow!(
                    "ActiveState of systemd unit [{}] cannot be retrieved: {}",
                    self.file,
                    error
                )
            })
    }

    /// Checks if the result is not set to success.
    pub async fn failed(&self) -> anyhow::Result<bool> {
        self.service_proxy
            .result()
            .await
            .map(|state| state != ServiceResult::Success)
            .map_err(|error| {
                anyhow!(
                    "Result of systemd unit [{}] cannot be retrieved: {}",
                    self.file,
                    error
                )
            })
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
