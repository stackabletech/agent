use std::collections::HashMap;

use super::running::Running;
use crate::provider::{
    kubernetes::status::patch_container_status, systemdmanager::service::ServiceState, PodHandle,
    PodState, ProviderState,
};

use anyhow::Result;
use kube::{
    api::{Patch, PatchParams},
    Api, Client,
};
use kubelet::pod::{Pod, PodKey};
use kubelet::{container::Status, pod::state::prelude::*};
use log::{debug, error, info};
use serde_json::json;

#[derive(Default, Debug, TransitionTo)]
#[transition_to(Running)]
pub struct Starting;

#[async_trait::async_trait]
impl State<PodState> for Starting {
    async fn next(
        self: Box<Self>,
        shared: SharedState<ProviderState>,
        pod_state: &mut PodState,
        pod: Manifest<Pod>,
    ) -> Transition<PodState> {
        let pod = pod.latest();

        match start_service_units(shared, pod_state, &pod).await {
            Ok(()) => Transition::next(self, Running::default()),
            Err(error) => {
                error!("{}", error);
                Transition::Complete(Err(error))
            }
        }
    }

    async fn status(&self, _pod_state: &mut PodState, _pod: &Pod) -> Result<PodStatus> {
        Ok(make_status(Phase::Pending, "Starting"))
    }
}

/// Starts the service units for the containers of the given pod.
///
/// The units are started and enabled if they were not already started.
async fn start_service_units(
    shared: SharedState<ProviderState>,
    pod_state: &PodState,
    pod: &Pod,
) -> Result<()> {
    let pod_key = &PodKey::from(pod);

    let (client, systemd_manager, pod_handle) = {
        let provider_state = shared.read().await;
        let handles = provider_state.handles.read().await;
        (
            provider_state.client.clone(),
            provider_state.systemd_manager.clone(),
            handles.get(pod_key).map(PodHandle::to_owned),
        )
    };

    for (container_key, container_handle) in pod_handle.unwrap_or_default() {
        let systemd_service = &container_handle.systemd_service;
        let service_unit = &container_handle.service_unit;

        if systemd_service.service_state().await? == ServiceState::Created {
            info!("Starting systemd unit [{}]", service_unit);
            systemd_manager.start(service_unit).await?;

            info!("Enabling systemd unit [{}]", service_unit);
            systemd_manager.enable(service_unit).await?;
        } else {
            debug!(
                "Unit [{}] for service [{}] was already started. Skipping startup.",
                service_unit, &pod_state.service_name
            );
        }

        let mut annotations = HashMap::new();
        annotations.insert(
            "featureLogs",
            systemd_service.invocation_id().await.is_ok().to_string(),
        );
        annotations.insert(
            "featureRestartCount",
            systemd_service.restart_count().await.is_ok().to_string(),
        );

        add_annotations(&client, pod, &annotations).await?;

        patch_container_status(&client, pod, &container_key, &Status::running()).await;
    }

    Ok(())
}

/// Adds annotations to the given pod.
///
/// If there is already an annotation with the given key then the value
/// is replaced.
/// The function returns when the patch is sent. It does not await the
/// changes to be visible to the watching clients.
async fn add_annotations(
    client: &Client,
    pod: &Pod,
    annotations: &HashMap<&str, String>,
) -> kube::Result<Pod> {
    debug!(
        "Adding annotations [{:?}] to pod [{:?}]",
        annotations,
        PodKey::from(pod)
    );

    let api: Api<Pod> = Api::namespaced(client.clone(), pod.namespace());

    let patch = json!({
        "metadata": {
            "annotations": annotations
        }
    });

    api.patch(
        pod.name(),
        &PatchParams::default(),
        &Patch::Strategic(patch),
    )
    .await
}
