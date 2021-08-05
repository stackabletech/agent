//! Functions for patching the pod status

use anyhow::anyhow;
use k8s_openapi::api::core::v1::Pod as KubePod;
use kube::{
    api::{Patch, PatchParams},
    Api, Client,
};
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

/// Patches the restart count of a container.
pub async fn patch_restart_count(
    client: &Client,
    pod: &Pod,
    container_key: &ContainerKey,
    restart_count: u32,
) -> anyhow::Result<()> {
    let api: Api<KubePod> = Api::namespaced(client.clone(), pod.namespace());

    let index = pod
        .container_status_index(container_key)
        .ok_or_else(|| anyhow!("Container not found"))?;

    let container_type = if container_key.is_init() {
        "initContainer"
    } else {
        "container"
    };

    let patch = json_patch::Patch(vec![json_patch::PatchOperation::Replace(
        json_patch::ReplaceOperation {
            path: format!("/status/{}Statuses/{}/restartCount", container_type, index),
            value: restart_count.into(),
        },
    )]);

    api.patch_status(
        pod.name(),
        &PatchParams::default(),
        &Patch::<()>::Json(patch),
    )
    .await?;

    Ok(())
}
