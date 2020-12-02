use std::convert::TryFrom;
use std::fs;
use std::path::PathBuf;
use std::process::Child;

use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::{Api, Client};
use kubelet::backoff::ExponentialBackoffStrategy;
use kubelet::log::Sender;
use kubelet::node::Builder;
use kubelet::pod::Pod;
use kubelet::provider::Provider;
use log::{debug, error};

use crate::provider::error::StackableError;
use crate::provider::error::StackableError::{CrdMissing, KubeError, PodValidationError};
use crate::provider::repository::package::Package;
use crate::provider::states::downloading::Downloading;
use crate::provider::states::terminated::Terminated;
use kube::error::ErrorResponse;

pub struct StackableProvider {
    client: Client,
    parcel_directory: PathBuf,
    config_directory: PathBuf,
    log_directory: PathBuf,
}

pub const CRDS: &[&str] = &["repositories.stable.stackable.de"];

mod error;
mod repository;
mod states;

pub struct PodState {
    client: Client,
    parcel_directory: PathBuf,
    download_directory: PathBuf,
    config_directory: PathBuf,
    log_directory: PathBuf,
    package_download_backoff_strategy: ExponentialBackoffStrategy,
    service_name: String,
    package: Package,
    process_handle: Option<Child>,
}

impl PodState {
    pub fn get_service_config_directory(&self) -> PathBuf {
        self.config_directory.join(&self.service_name)
    }

    pub fn get_service_package_directory(&self) -> PathBuf {
        self.parcel_directory
            .join(&self.package.get_directory_name())
    }

    pub fn get_service_log_directory(&self) -> PathBuf {
        self.log_directory.join(&self.service_name)
    }
}

impl StackableProvider {
    pub async fn new(
        client: Client,
        parcel_directory: PathBuf,
        config_directory: PathBuf,
        log_directory: PathBuf,
    ) -> Result<Self, StackableError> {
        let provider = StackableProvider {
            client,
            parcel_directory,
            config_directory,
            log_directory,
        };
        let missing_crds = provider.check_crds().await?;
        return if missing_crds.is_empty() {
            debug!("All required CRDS present!");
            Ok(provider)
        } else {
            debug!("Missing required CDRS: [{:?}]", &missing_crds);
            Err(CrdMissing { missing_crds })
        };
    }

    fn get_package(pod: &Pod) -> Result<Package, StackableError> {
        let containers = pod.containers();
        return if containers.len().ne(&1) {
            let e = PodValidationError {
                msg: String::from("Size of containers list in PodSpec has to be exactly 1"),
            };
            Err(e)
        } else {
            // List has exactly one value, try to parse this
            if let Ok(Some(reference)) = containers[0].image() {
                Package::try_from(reference)
            } else {
                let e = PodValidationError {
                    msg: format!("Unable to get package reference from pod: {}", &pod.name()),
                };
                Err(e)
            }
        };
    }

    async fn check_crds(&self) -> Result<Vec<String>, StackableError> {
        let mut missing_crds = vec![];
        let crds: Api<CustomResourceDefinition> = Api::all(self.client.clone());

        // Check all CRDS
        for crd in CRDS.into_iter() {
            debug!("Checking if CRD [{}] is registered", crd);
            match crds.get(crd).await {
                Err(kube::error::Error::Api(ErrorResponse { reason, .. }))
                    if reason == "NotFound" =>
                {
                    error!("Missing required CRD: [{}]", crd);
                    missing_crds.push(String::from(*crd))
                }
                Err(e) => {
                    error!(
                        "An error ocurred when checking if CRD [{}] is registered: \"{}\"",
                        crd, e
                    );
                    return Err(KubeError { source: e });
                }
                _ => debug!("Found registered crd: [{}]", crd),
            }
        }
        Ok(missing_crds)
    }
}

// No cleanup state needed, we clean up when dropping PodState.
#[async_trait::async_trait]
impl kubelet::state::AsyncDrop for PodState {
    async fn async_drop(self) {}
}

#[async_trait::async_trait]
impl Provider for StackableProvider {
    type PodState = PodState;
    type InitialState = Downloading;
    type TerminatedState = Terminated;

    const ARCH: &'static str = "stackable-linux";

    async fn node(&self, builder: &mut Builder) -> anyhow::Result<()> {
        builder.set_architecture(Self::ARCH);
        builder.add_taint("NoSchedule", "kubernetes.io/arch", Self::ARCH);
        builder.add_taint("NoExecute", "kubernetes.io/arch", Self::ARCH);
        Ok(())
    }

    async fn initialize_pod_state(&self, pod: &Pod) -> anyhow::Result<Self::PodState> {
        let service_name = pod.name();
        let parcel_directory = self.parcel_directory.clone();
        // TODO: make this configurable
        let download_directory = parcel_directory.join("_download");
        let config_directory = self.config_directory.clone();
        let log_directory = self.log_directory.clone();

        let package = Self::get_package(pod)?;
        if !(&download_directory.is_dir()) {
            fs::create_dir_all(&download_directory)?;
        }
        if !(&config_directory.is_dir()) {
            fs::create_dir_all(&config_directory)?;
        }

        Ok(PodState {
            client: self.client.clone(),
            parcel_directory,
            download_directory,
            log_directory,
            config_directory: self.config_directory.clone(),
            package_download_backoff_strategy: ExponentialBackoffStrategy::default(),
            service_name: String::from(service_name),
            package,
            process_handle: None,
        })
    }

    async fn logs(
        &self,
        _namespace: String,
        _pod: String,
        _container: String,
        _sender: Sender,
    ) -> anyhow::Result<()> {
        Ok(())
    }
}
