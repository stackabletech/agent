use std::convert::TryFrom;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::anyhow;
use k8s_openapi::apiextensions_apiserver::pkg::apis::apiextensions::v1::CustomResourceDefinition;
use kube::{Api, Client};
use kubelet::backoff::ExponentialBackoffStrategy;
use kubelet::log::{SendError, Sender};
use kubelet::node::Builder;
use kubelet::pod::state::prelude::*;
use kubelet::pod::{Pod, PodKey};
use kubelet::{
    container::{ContainerKey, ContainerMap},
    provider::Provider,
};
use log::{debug, error, info};
use tokio::{runtime::Runtime, sync::RwLock, task};

use crate::provider::error::StackableError;
use crate::provider::error::StackableError::{
    CrdMissing, KubeError, MissingObjectKey, PodValidationError,
};
use crate::provider::repository::package::Package;
use crate::provider::states::pod::downloading::Downloading;
use crate::provider::states::pod::terminated::Terminated;
use crate::provider::states::pod::PodState;
use crate::provider::systemdmanager::manager::SystemdManager;
use kube::error::ErrorResponse;
use std::{collections::HashMap, time::Duration};
use systemdmanager::journal_reader;

pub struct StackableProvider {
    shared: ProviderState,
    parcel_directory: PathBuf,
    config_directory: PathBuf,
    log_directory: PathBuf,
    pod_cidr: String,
}

pub const CRDS: &[&str] = &["repositories.stable.stackable.de"];

mod error;
mod repository;
mod states;
mod systemdmanager;

#[derive(Clone, Debug)]
pub struct ContainerHandle {
    pub service_unit: String,
    pub invocation_id: Option<String>,
}

impl ContainerHandle {
    pub fn new(service_unit: &str) -> Self {
        ContainerHandle {
            service_unit: String::from(service_unit),
            invocation_id: None,
        }
    }
}

type PodHandle = ContainerMap<ContainerHandle>;

/// Provider-level state shared between all pods
#[derive(Clone)]
pub struct ProviderState {
    handles: Arc<RwLock<PodHandleMap>>,
    client: Client,
    systemd_manager: Arc<SystemdManager>,
}

#[derive(Debug, Default)]
struct PodHandleMap {
    handles: HashMap<PodKey, PodHandle>,
}

impl PodHandleMap {
    pub fn get(&self, pod_key: &PodKey) -> Option<&PodHandle> {
        self.handles.get(pod_key)
    }

    pub fn remove(&mut self, pod_key: &PodKey) -> Option<PodHandle> {
        self.handles.remove(pod_key)
    }

    pub fn insert_container_handle(
        &mut self,
        pod_key: &PodKey,
        container_key: &ContainerKey,
        service_unit: &str,
    ) {
        self.handles
            .entry(pod_key.to_owned())
            .or_insert_with(ContainerMap::new)
            .insert(container_key.to_owned(), ContainerHandle::new(service_unit));
        info!("Handles inserted: {:?}", self.handles);
    }

    pub fn set_invocation_id(
        &mut self,
        pod_key: &PodKey,
        container_key: &ContainerKey,
        invocation_id: &str,
    ) -> anyhow::Result<()> {
        if let Some(mut container_handle) = self.container_handle_mut(pod_key, container_key) {
            container_handle.invocation_id = Some(String::from(invocation_id));
            Ok(())
        } else {
            Err(anyhow!("Container handle not found"))
        }
    }

    pub fn container_handle(
        &self,
        pod_key: &PodKey,
        container_key: &ContainerKey,
    ) -> Option<&ContainerHandle> {
        self.handles
            .get(pod_key)
            .and_then(|pod_handle| pod_handle.get(container_key))
    }

    fn container_handle_mut(
        &mut self,
        pod_key: &PodKey,
        container_key: &ContainerKey,
    ) -> Option<&mut ContainerHandle> {
        self.handles
            .get_mut(pod_key)
            .and_then(|pod_handle| pod_handle.get_mut(container_key))
    }
}

impl StackableProvider {
    pub async fn new(
        client: Client,
        parcel_directory: PathBuf,
        config_directory: PathBuf,
        log_directory: PathBuf,
        session: bool,
        pod_cidr: String,
    ) -> Result<Self, StackableError> {
        let systemd_manager = Arc::new(SystemdManager::new(session, Duration::from_secs(5))?);

        let provider_state = ProviderState {
            handles: Default::default(),
            client,
            systemd_manager,
        };

        let provider = StackableProvider {
            shared: provider_state,
            parcel_directory,
            config_directory,
            log_directory,
            pod_cidr,
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
        let crds: Api<CustomResourceDefinition> = Api::all(self.shared.client.clone());

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

#[async_trait::async_trait]
impl Provider for StackableProvider {
    type ProviderState = ProviderState;
    type PodState = PodState;
    type InitialState = Downloading;
    type TerminatedState = Terminated;

    const ARCH: &'static str = "stackable-linux";

    fn provider_state(&self) -> SharedState<ProviderState> {
        Arc::new(RwLock::new(self.shared.clone()))
    }

    async fn node(&self, builder: &mut Builder) -> anyhow::Result<()> {
        builder.set_architecture(Self::ARCH);
        builder.set_pod_cidr(&self.pod_cidr);
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

        let package = Self::get_package(pod)?;
        if !(&download_directory.is_dir()) {
            fs::create_dir_all(&download_directory)?;
        }
        if !(&config_directory.is_dir()) {
            fs::create_dir_all(&config_directory)?;
        }

        Ok(PodState {
            parcel_directory,
            download_directory,
            log_directory,
            config_directory: self.config_directory.clone(),
            package_download_backoff_strategy: ExponentialBackoffStrategy::default(),
            service_name,
            service_uid,
            package,
        })
    }

    async fn logs(
        &self,
        namespace: String,
        pod: String,
        container: String,
        mut sender: Sender,
    ) -> anyhow::Result<()> {
        info!("Logs requested");

        info!("Shared state handles: {:?}", self.shared.handles);

        let handles = self.shared.handles.read().await;

        let pod_key = PodKey::new(&namespace, &pod);
        let container_key = ContainerKey::App(container);
        let container_handle = handles
            .container_handle(&pod_key, &container_key)
            .ok_or_else(|| {
                anyhow!(
                    "Container handle for pod [{:?}] and container [{:?}] not found",
                    pod_key,
                    container_key
                )
            })?;
        let invocation_id = container_handle.invocation_id.to_owned().ok_or_else(|| {
            anyhow!(
                "Invocation ID for container [{}] in pod [{:?}] is unknown. \
                    The service is probably not started yet.",
                container_key,
                pod_key
            )
        })?;

        task::spawn_blocking(move || {
            let result = Runtime::new()
                .unwrap()
                .block_on(journal_reader::send_journal_entries(
                    &mut sender,
                    &invocation_id,
                ));

            if let Err(error) = result {
                match error.downcast_ref::<SendError>() {
                    Some(SendError::ChannelClosed) => (),
                    _ => error!("Log could not be sent. {}", error),
                }
            }
        });

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

    #[rstest]
    #[case(indoc! {"
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
    )]
    #[case(indoc! {"
            apiVersion: v1
            kind: Pod
            metadata:
              name: test
            spec:
              containers:
              - name: kafka
        "},
        "Unable to get package reference from pod [test]: Image is required."
    )]
    #[case(indoc! {"
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
    )]
    fn try_to_get_package_from_insufficient_configuration(
        #[case] pod_config: &str,
        #[case] expected_err: &str,
    ) {
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
