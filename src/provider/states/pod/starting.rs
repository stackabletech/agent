use super::running::Running;
use crate::provider::{
    kubernetes::{
        accessor::{restart_policy, RestartPolicy},
        status::patch_container_status,
    },
    systemdmanager::service::{ServiceState, SystemdService},
    PodHandle, PodState, ProviderState,
};

use anyhow::{anyhow, Result};
use kube::{
    api::{Patch, PatchParams},
    Api, Client,
};
use kubelet::pod::{Pod, PodKey};
use kubelet::{container::Status, pod::state::prelude::*};
use log::{debug, error, info};
use serde_json::json;
use std::time::Instant;
use tokio::time::{self, Duration};

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
/// The units are started and enabled if they are not already running.
/// The startup is considered successful if the unit is still running
/// after 10 seconds.
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

        if systemd_service.service_state().await? == ServiceState::Started {
            debug!(
                "Unit [{}] for service [{}] is already running. Skip startup.",
                systemd_service.file(),
                &pod_state.service_name
            );
        } else {
            let service_unit = &container_handle.service_unit;

            info!("Starting systemd unit [{}]", service_unit);
            systemd_manager.start(service_unit).await?;

            info!("Enabling systemd unit [{}]", service_unit);
            systemd_manager.enable(service_unit).await?;

            if restart_policy(pod) == RestartPolicy::Always {
                // TODO: does this need to be configurable, or ar we happy with a hard coded value
                //  for now. I've briefly looked at the podspec and couldn't identify a good field
                //  to use for this - also, currently this starts containers (= systemd units) in
                //  order and waits 10 seconds for every unit, so a service with five containers
                //  would take 50 seconds until it reported running - which is totally fine in case
                //  the units actually depend on each other, but a case could be made for waiting
                //  once at the end
                await_startup(systemd_service, Duration::from_secs(10)).await?;
            }
        }

        add_annotation(
            &client,
            pod,
            "featureLogs",
            &systemd_service.invocation_id().await.is_ok().to_string(),
        )
        .await?;

        patch_container_status(&client, pod, &container_key, &Status::running()).await;
    }

    Ok(())
}

/// Checks if the given service unit is still running after the given duration.
async fn await_startup(systemd_service: &SystemdService, duration: Duration) -> Result<()> {
    let start_time = Instant::now();
    while start_time.elapsed() < duration {
        time::sleep(Duration::from_secs(1)).await;

        debug!(
            "Checking if unit [{}] is still up and running.",
            systemd_service.file()
        );

        if systemd_service.service_state().await? == ServiceState::Started {
            debug!(
                "Service [{}] still running after [{}] seconds",
                systemd_service.file(),
                start_time.elapsed().as_secs()
            );
        } else {
            return Err(anyhow!(
                "Unit [{}] stopped unexpectedly during startup after [{}] seconds.",
                systemd_service.file(),
                start_time.elapsed().as_secs()
            ));
        }
    }

    Ok(())
}

/// Adds an annotation to the given pod.
///
/// If there is already an annotation with the given key then the value
/// is replaced.
/// The function returns when the patch is sent. It does not await the
/// changes to be visible to the watching clients.
async fn add_annotation(client: &Client, pod: &Pod, key: &str, value: &str) -> kube::Result<Pod> {
    debug!(
        "Adding annotation [{}: {}] to pod [{:?}]",
        key,
        value,
        PodKey::from(pod)
    );

    let api: Api<Pod> = Api::namespaced(client.clone(), pod.namespace());

    let patch = json!({
        "metadata": {
            "annotations": {
                key: value
            }
        }
    });

    api.patch(
        pod.name(),
        &PatchParams::default(),
        &Patch::Strategic(patch),
    )
    .await
}
