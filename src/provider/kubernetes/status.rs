//! Functions for patching the pod status

use k8s_openapi::api::core::v1::Pod as KubePod;
use kube::{Api, Client};
use kubelet::{
    container::{ContainerKey, Status},
    pod::Pod,
};
use log::warn;

/// Patches the pod status with the given container status.
///
/// If the patching fails then a warning is logged.
pub async fn patch_container_status(
    client: &Client,
    pod: &Pod,
    container_key: &ContainerKey,
    status: &Status,
) {
    let api: Api<KubePod> = Api::namespaced(client.clone(), pod.namespace());

    if let Err(error) =
        kubelet::container::patch_container_status(&api, pod, container_key, status).await
    {
        warn!(
            "Status of container [{}] in pod [{}] could not be patched. {}",
            container_key,
            pod.name(),
            error
        );
    }
}
