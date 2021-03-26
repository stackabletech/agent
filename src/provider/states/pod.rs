use std::path::PathBuf;

use kubelet::backoff::ExponentialBackoffStrategy;
use kubelet::pod::state::prelude::*;
use kubelet::pod::{Pod, Status};

use crate::provider::repository::package::Package;
use crate::provider::systemdmanager::systemdunit::SystemDUnit;
use crate::provider::ProviderState;

pub(crate) mod creating_config;
pub(crate) mod creating_service;
pub(crate) mod downloading;
pub(crate) mod downloading_backoff;
pub(crate) mod failed;
pub(crate) mod installing;
pub(crate) mod running;
pub(crate) mod setup_failed;
pub(crate) mod starting;
pub(crate) mod terminated;
pub(crate) mod waiting_config_map;

pub struct PodState {
    pub parcel_directory: PathBuf,
    pub download_directory: PathBuf,
    pub config_directory: PathBuf,
    pub log_directory: PathBuf,
    pub package_download_backoff_strategy: ExponentialBackoffStrategy,
    pub service_name: String,
    pub service_uid: String,
    pub package: Package,
    pub service_units: Option<Vec<SystemDUnit>>,
}

impl PodState {
    pub fn get_service_config_directory(&self) -> PathBuf {
        self.config_directory
            .join(format!("{}-{}", &self.service_name, &self.service_uid))
    }

    pub fn get_service_package_directory(&self) -> PathBuf {
        self.parcel_directory
            .join(&self.package.get_directory_name())
    }

    pub fn get_service_log_directory(&self) -> PathBuf {
        self.log_directory.join(&self.service_name)
    }

    /// Resolve the directory in which the systemd unit files will be placed for this
    /// service.
    /// This defaults to "{{config_root}}/_service"
    ///
    /// From this place the unit files will be symlinked to the relevant systemd
    /// unit directories so that they are picked up by systemd.
    pub fn get_service_service_directory(&self) -> PathBuf {
        self.get_service_config_directory().join("_service")
    }
}

// No cleanup state needed, we clean up when dropping PodState.
#[async_trait::async_trait]
impl ObjectState for PodState {
    type Manifest = Pod;
    type Status = Status;
    type SharedState = ProviderState;

    async fn async_drop(self, _provider_state: &mut ProviderState) {}
}
