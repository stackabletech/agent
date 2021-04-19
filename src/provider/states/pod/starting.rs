use super::failed::Failed;
use super::running::Running;
use super::setup_failed::SetupFailed;
use crate::provider::{
    systemdmanager::manager::SystemdManager, ContainerHandle, PodHandle, PodState, ProviderState,
};

use anyhow::anyhow;
use kubelet::pod::state::prelude::*;
use kubelet::{
    container::ContainerKey,
    pod::{Pod, PodKey},
};
use log::{debug, error, info};
use std::time::Instant;
use tokio::time::{self, Duration};

#[derive(Default, Debug, TransitionTo)]
#[transition_to(Running, Failed, SetupFailed)]
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

        match start_containers(shared, pod_state, &pod).await {
            Ok(()) => Transition::next(
                self,
                Running {
                    ..Default::default()
                },
            ),
            Err(error) => {
                error!("{}", error);
                Transition::Complete(Err(error))
            }
        }
    }

    async fn status(&self, _pod_state: &mut PodState, _pod: &Pod) -> anyhow::Result<PodStatus> {
        Ok(make_status(Phase::Pending, &"Starting"))
    }
}

async fn start_containers(
    shared: SharedState<ProviderState>,
    pod_state: &PodState,
    pod: &Pod,
) -> anyhow::Result<()> {
    let pod_key = &PodKey::from(pod);

    let (systemd_manager, pod_handle) = {
        let provider_state = shared.read().await;
        let handles = provider_state.handles.read().await;
        (
            provider_state.systemd_manager.clone(),
            handles.get(&pod_key).map(PodHandle::to_owned),
        )
    };

    for (container_key, container_handle) in pod_handle.unwrap_or_default() {
        if systemd_manager.is_running(&container_handle.service_unit)? {
            debug!(
                "Unit [{}] for service [{}] already running, nothing to do..",
                &container_handle.service_unit, &pod_state.service_name
            );
        } else {
            info!("Starting systemd unit [{}]", container_handle.service_unit);
            systemd_manager.start(&container_handle.service_unit)?;

            info!("Enabling systemd unit [{}]", container_handle.service_unit);
            systemd_manager.enable(&container_handle.service_unit)?;

            await_startup(&systemd_manager, &container_handle).await?;
        }

        let invocation_id = systemd_manager.get_invocation_id(&container_handle.service_unit)?;
        enter_invocation_id(shared.clone(), pod_key, &container_key, &invocation_id).await?;
    }

    Ok(())
}

async fn await_startup(
    systemd_manager: &SystemdManager,
    container_handle: &ContainerHandle,
) -> anyhow::Result<()> {
    let start_time = Instant::now();
    // TODO: does this need to be configurable, or ar we happy with a hard coded value
    //  for now. I've briefly looked at the podspec and couldn't identify a good field
    //  to use for this - also, currently this starts containers (= systemd units) in
    //  order and waits 10 seconds for every unit, so a service with five containers
    //  would take 50 seconds until it reported running - which is totally fine in case
    //  the units actually depend on each other, but a case could be made for waiting
    //  once at the end
    while start_time.elapsed().as_secs() < 10 {
        time::sleep(Duration::from_secs(1)).await;

        debug!(
            "Checking if unit [{}] is still up and running.",
            container_handle.service_unit
        );

        if systemd_manager.is_running(&container_handle.service_unit)? {
            debug!(
                "Service [{}] still running after [{}] seconds",
                &container_handle.service_unit,
                start_time.elapsed().as_secs()
            );
        } else {
            return Err(anyhow!(
                "Unit [{}] stopped unexpectedly during startup after [{}] seconds.",
                &container_handle.service_unit,
                start_time.elapsed().as_secs()
            ));
        }
    }

    Ok(())
}

async fn enter_invocation_id(
    shared: SharedState<ProviderState>,
    pod_key: &PodKey,
    container_key: &ContainerKey,
    invocation_id: &str,
) -> anyhow::Result<()> {
    debug!(
        "Set invocation ID [{}] for pod [{:?}] and container [{}].",
        invocation_id, pod_key, container_key
    );

    let provider_state = shared.write().await;
    let mut handles = provider_state.handles.write().await;
    handles.set_invocation_id(&pod_key, &container_key, &invocation_id)
}
