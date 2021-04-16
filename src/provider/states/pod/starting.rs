use kubelet::pod::state::prelude::*;
use kubelet::pod::{Pod, PodKey};

use super::failed::Failed;
use super::running::Running;
use super::setup_failed::SetupFailed;
use crate::provider::{PodHandle, PodState, ProviderState};
use anyhow::anyhow;
use log::{debug, error, info, warn};
use std::time::Instant;
use tokio::time::Duration;

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
        let pod_key = &PodKey::from(pod);

        let (systemd_manager, pod_handle) = {
            let provider_state = shared.read().await;
            (
                provider_state.systemd_manager.clone(),
                provider_state
                    .handles
                    .get(&pod_key)
                    .map(PodHandle::to_owned),
            )
        };

        if let Some(containers) = pod_handle {
            for (container_key, container_handle) in containers {
                match systemd_manager.is_running(&container_handle.service_unit) {
                    Ok(true) => {
                        debug!(
                            "Unit [{}] for service [{}] already running, nothing to do..",
                            &container_handle.service_unit, &pod_state.service_name
                        );
                        // Skip rest of loop as the service is already running
                        continue;
                    }
                    Err(dbus_error) => {
                        debug!(
                            "Error retrieving activestate of unit [{}] for service [{}]: [{}]",
                            &container_handle.service_unit, &pod_state.service_name, dbus_error
                        );
                        return Transition::Complete(Err(dbus_error));
                    }
                    _ => { // nothing to do, just keep going
                    }
                }
                info!("Starting systemd unit [{}]", container_handle.service_unit);
                if let Err(start_error) = systemd_manager.start(&container_handle.service_unit) {
                    error!(
                        "Error occurred starting systemd unit [{}]: [{}]",
                        container_handle.service_unit, start_error
                    );
                    return Transition::Complete(Err(start_error));
                }

                info!("Enabling systemd unit [{}]", container_handle.service_unit);
                if let Err(enable_error) = systemd_manager.enable(&container_handle.service_unit) {
                    error!(
                        "Error occurred starting systemd unit [{}]: [{}]",
                        container_handle.service_unit, enable_error
                    );
                    return Transition::Complete(Err(enable_error));
                }

                let start_time = Instant::now();
                // TODO: does this need to be configurable, or ar we happy with a hard coded value
                //  for now. I've briefly looked at the podspec and couldn't identify a good field
                //  to use for this - also, currently this starts containers (= systemd units) in
                //  order and waits 10 seconds for every unit, so a service with five containers
                //  would take 50 seconds until it reported running - which is totally fine in case
                //  the units actually depend on each other, but a case could be made for waiting
                //  once at the end
                while start_time.elapsed().as_secs() < 10 {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    debug!(
                        "Checking if unit [{}] is still up and running.",
                        container_handle.service_unit
                    );
                    match systemd_manager.is_running(&container_handle.service_unit) {
                        Ok(true) => debug!(
                            "Service [{}] still running after [{}] seconds",
                            &container_handle.service_unit,
                            start_time.elapsed().as_secs()
                        ),
                        Ok(false) => {
                            return Transition::Complete(Err(anyhow!(
                                "Unit [{}] stopped unexpectedly during startup after [{}] seconds.",
                                &container_handle.service_unit,
                                start_time.elapsed().as_secs()
                            )))
                        }
                        Err(dbus_error) => return Transition::Complete(Err(dbus_error)),
                    }
                }

                info!("Creating container handle");
                let invocation_id =
                    match systemd_manager.get_invocation_id(&container_handle.service_unit) {
                        Ok(invocation_id) => invocation_id,
                        Err(dbus_error) => return Transition::Complete(Err(dbus_error)),
                    };

                {
                    let mut provider_state = shared.write().await;
                    if provider_state
                        .set_invocation_id(&pod_key, &container_key, &invocation_id)
                        .is_err()
                    {
                        return Transition::Complete(Err(anyhow!(
                            "Container [{}] in pod [{:?}] not found",
                            container_key,
                            pod_key
                        )));
                    }
                }
            }
        } else {
            warn!(
                "No unit definitions found, not starting anything for pod [{}]!",
                pod_state.service_name
            );
        }
        Transition::next(
            self,
            Running {
                ..Default::default()
            },
        )
    }

    async fn status(&self, _pod_state: &mut PodState, _pod: &Pod) -> anyhow::Result<PodStatus> {
        Ok(make_status(Phase::Pending, &"Starting"))
    }
}
