use kubelet::pod::state::prelude::*;
use kubelet::{
    container::ContainerKey,
    pod::{Pod, PodKey},
};

use super::failed::Failed;
use super::running::Running;
use super::setup_failed::SetupFailed;
use crate::provider::{ContainerHandle, PodHandle, PodState, ProviderState};
use anyhow::anyhow;
use log::{debug, error, info, warn};
use std::{collections::HashMap, time::Instant};
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
        let systemd_manager = {
            let provider_state = shared.read().await;
            provider_state.systemd_manager.clone()
        };

        if let Some(systemd_units) = &pod_state.service_units {
            for unit in systemd_units {
                match systemd_manager.is_running(&unit.get_name()) {
                    Ok(true) => {
                        debug!(
                            "Unit [{}] for service [{}] already running, nothing to do..",
                            &unit.get_name(),
                            &pod_state.service_name
                        );
                        // Skip rest of loop as the service is already running
                        continue;
                    }
                    Err(dbus_error) => {
                        debug!(
                            "Error retrieving activestate of unit [{}] for service [{}]: [{}]",
                            &unit.get_name(),
                            &pod_state.service_name,
                            dbus_error
                        );
                        return Transition::Complete(Err(dbus_error));
                    }
                    _ => { // nothing to do, just keep going
                    }
                }
                info!("Starting systemd unit [{}]", unit);
                if let Err(start_error) = systemd_manager.start(&unit.get_name()) {
                    error!(
                        "Error occurred starting systemd unit [{}]: [{}]",
                        unit.get_name(),
                        start_error
                    );
                    return Transition::Complete(Err(start_error));
                }

                info!("Enabling systemd unit [{}]", unit);
                if let Err(enable_error) = systemd_manager.enable(&unit.get_name()) {
                    error!(
                        "Error occurred starting systemd unit [{}]: [{}]",
                        unit.get_name(),
                        enable_error
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
                        &unit.get_name()
                    );
                    match systemd_manager.is_running(&unit.get_name()) {
                        Ok(true) => debug!(
                            "Service [{}] still running after [{}] seconds",
                            &unit.get_name(),
                            start_time.elapsed().as_secs()
                        ),
                        Ok(false) => {
                            return Transition::Complete(Err(anyhow!(format!(
                                "Unit [{}] stopped unexpectedly during startup after [{}] seconds.",
                                &unit.get_name(),
                                start_time.elapsed().as_secs()
                            ))))
                        }
                        Err(dbus_error) => return Transition::Complete(Err(dbus_error)),
                    }
                }

                info!("Creating container handle");
                {
                    let pod = pod.latest();
                    let provider_state = shared.write().await;
                    let pod_key = PodKey::from(pod.clone());
                    info!("Pod [{:?}] inserted into handles", pod_key);
                    let mut handles_writer = provider_state.handles.write().await;
                    let pod_handle = handles_writer.entry(pod_key).or_insert_with(|| PodHandle {
                        containers: HashMap::new(),
                    });
                    match systemd_manager.get_invocation_id(&unit.get_name()) {
                        Ok(invocation_id) => {
                            pod_handle.containers.insert(
                                ContainerKey::App(
                                    unit.container_name
                                        .as_ref()
                                        .expect("Container name is missing")
                                        .clone(),
                                ),
                                ContainerHandle { invocation_id },
                            );
                        }
                        Err(dbus_error) => return Transition::Complete(Err(dbus_error)),
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
