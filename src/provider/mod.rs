use std::convert::TryFrom;
use std::fs;
use std::path::PathBuf;

use anyhow::anyhow;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::{Api, Client};
use kubelet::backoff::ExponentialBackoffStrategy;
use kubelet::log::Sender;
use kubelet::node::Builder;
use kubelet::pod::Pod;
use kubelet::provider::Provider;
use log::{debug, error};

use crate::provider::error::StackableError;
use crate::provider::error::StackableError::{
    CrdMissing, KubeError, MissingObjectKey, PodValidationError,
};
use crate::provider::repository::package::Package;
use crate::provider::states::downloading::Downloading;
use crate::provider::states::terminated::Terminated;
use crate::provider::systemdmanager::manager::SystemdManager;
use crate::provider::systemdmanager::service::Service;
use kube::error::ErrorResponse;
use std::time::Duration;

pub struct StackableProvider {
    client: Client,
    parcel_directory: PathBuf,
    config_directory: PathBuf,
    log_directory: PathBuf,
    session: bool,
}

pub const CRDS: &[&str] = &["repositories.stable.stackable.de"];

mod error;
mod repository;
mod states;
mod systemdmanager;

pub struct PodState {
    client: Client,
    parcel_directory: PathBuf,
    download_directory: PathBuf,
    config_directory: PathBuf,
    log_directory: PathBuf,
    package_download_backoff_strategy: ExponentialBackoffStrategy,
    service_name: String,
    service_uid: String,
    package: Package,
    systemd_manager: SystemdManager,
    service_units: Option<Service>,
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

impl StackableProvider {
    pub async fn new(
        client: Client,
        parcel_directory: PathBuf,
        config_directory: PathBuf,
        log_directory: PathBuf,
        session: bool,
    ) -> Result<Self, StackableError> {
        let provider = StackableProvider {
            client,
            parcel_directory,
            config_directory,
            log_directory,
            session,
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
        if let Some((container, [])) = pod.containers().split_first() {
            container
                .image()
                .and_then(|maybe_ref| maybe_ref.ok_or_else(|| anyhow!("Image is required.")))
                .and_then(Package::try_from)
                .map_err(|err| PodValidationError {
                    msg: format!(
                        "Unable to get package reference from pod [{}]: {}",
                        &pod.name(),
                        &err
                    ),
                })
        } else {
            Err(PodValidationError {
                msg: String::from("Only one container is supported in the PodSpec."),
            })
        }
    }

    async fn check_crds(&self) -> Result<Vec<String>, StackableError> {
        let mut missing_crds = vec![];
        let crds: Api<CustomResourceDefinition> = Api::all(self.client.clone());

        // Check all CRDS
        for crd in CRDS.iter() {
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
        let service_name = format!("{}-{}", pod.namespace(), pod.name());

        // Extract uid from pod object, if this fails we return an error -
        // this should not happen, as all objects we get from Kubernetes should have
        // a uid set!
        let service_uid = if let Some(uid) = pod.as_kube_pod().metadata.uid.as_ref() {
            uid.to_string()
        } else {
            return Err(anyhow::Error::new(MissingObjectKey {
                key: ".metadata.uid",
            }));
        };
        let parcel_directory = self.parcel_directory.clone();
        // TODO: make this configurable
        let download_directory = parcel_directory.join("_download");
        let config_directory = self.config_directory.clone();
        let log_directory = self.log_directory.clone();
        let session = self.session;

        let package = Self::get_package(pod)?;
        if !(&download_directory.is_dir()) {
            fs::create_dir_all(&download_directory)?;
        }
        if !(&config_directory.is_dir()) {
            fs::create_dir_all(&config_directory)?;
        }

        // TODO: investigate if we can share one DBus connection across all pods
        let systemd_manager = SystemdManager::new(session, Duration::from_secs(5))?;

        Ok(PodState {
            client: self.client.clone(),
            parcel_directory,
            download_directory,
            log_directory,
            config_directory: self.config_directory.clone(),
            package_download_backoff_strategy: ExponentialBackoffStrategy::default(),
            service_name,
            service_uid,
            package,
            // TODO: Check if we can work with a reference or a Mutex Guard here to only keep
            // one connection open to DBus instead of one per tracked Pod
            systemd_manager,
            service_units: None,
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

#[cfg(test)]
mod test {
    use super::*;
    use indoc::indoc;
    use rstest::rstest;

    #[test]
    fn try_to_get_package_from_complete_configuration() {
        let pod = parse_pod_from_yaml(indoc! {"
            apiVersion: v1
            kind: Pod
            metadata:
              name: test
            spec:
              containers:
              - name: kafka
                image: kafka:2.7
        "});

        let maybe_package = StackableProvider::get_package(&pod);

        if let Ok(package) = maybe_package {
            assert_eq!("kafka", package.product);
            assert_eq!("2.7", package.version);
        } else {
            panic!("Package expected but got {:?}", maybe_package);
        }
    }

    #[rstest(pod_config, expected_err,
        case(indoc! {"
                apiVersion: v1
                kind: Pod
                metadata:
                  name: test
                spec:
                  containers:
                  - name: kafka
                    image: kafka:2.7
                  - name: zookeeper
                    image: zookeeper:3.6.2
            "},
            "Only one container is supported in the PodSpec."
        ),
        case(indoc! {"
                apiVersion: v1
                kind: Pod
                metadata:
                  name: test
                spec:
                  containers:
                  - name: kafka
            "},
            "Unable to get package reference from pod [test]: Image is required."
        ),
        case(indoc! {"
                apiVersion: v1
                kind: Pod
                metadata:
                  name: test
                spec:
                  containers:
                  - name: kafka
                    image: kafka
            "},
            "Unable to get package reference from pod [test]: Tag is required."
        ),
    )]
    fn try_to_get_package_from_insufficient_configuration(pod_config: &str, expected_err: &str) {
        let pod = parse_pod_from_yaml(pod_config);

        let maybe_package = StackableProvider::get_package(&pod);

        if let Err(PodValidationError { msg }) = maybe_package {
            assert_eq!(expected_err, msg);
        } else {
            panic!("PodValidationError expected but got {:?}", maybe_package);
        }
    }

    fn parse_pod_from_yaml(pod_config: &str) -> Pod {
        let kube_pod: k8s_openapi::api::core::v1::Pod = serde_yaml::from_str(pod_config).unwrap();
        Pod::from(kube_pod)
    }
}
