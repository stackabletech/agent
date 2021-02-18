use kubelet::pod::Pod;
use kubelet::state::prelude::*;
use kubelet::state::{State, Transition};

use crate::provider::states::failed::Failed;
use crate::provider::states::running::Running;
use crate::provider::states::setup_failed::SetupFailed;
use crate::provider::PodState;
use anyhow::anyhow;
use log::{debug, error, info, warn};
use std::time::Instant;
use tokio::time::Duration;

#[derive(Default, Debug, TransitionTo)]
#[transition_to(Running, Failed, SetupFailed)]
pub struct Starting;

#[async_trait::async_trait]
impl State<PodState> for Starting {
    async fn next(self: Box<Self>, pod_state: &mut PodState, _: &Pod) -> Transition<PodState> {
        if let Some(systemd_units) = &pod_state.service_units {
            for unit in systemd_units {
                match pod_state.systemd_manager.is_running(&unit.get_name()) {
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
                        return Transition::Complete(Err(anyhow::Error::from(dbus_error)));
                    }
                    _ => { // nothing to do, just keep going
                    }
                }
                info!("Starting systemd unit [{}]", unit);
                if let Err(start_error) = pod_state.systemd_manager.start(&unit.get_name()) {
                    error!(
                        "Error occurred starting systemd unit [{}]: [{}]",
                        unit.get_name(),
                        start_error
                    );
                    return Transition::Complete(Err(start_error));
                }

                info!("Enabling systemd unit [{}]", unit);
                if let Err(enable_error) = pod_state.systemd_manager.enable(&unit.get_name()) {
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
                    tokio::time::delay_for(Duration::from_secs(1)).await;
                    debug!(
                        "Checking if unit [{}] is still up and running.",
                        &unit.get_name()
                    );
                    match pod_state.systemd_manager.is_running(&unit.get_name()) {
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

    async fn json_status(
        &self,
        _pod_state: &mut PodState,
        _pod: &Pod,
    ) -> anyhow::Result<serde_json::Value> {
        make_status(Phase::Pending, &"Starting")
    }
}
