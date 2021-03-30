use anyhow::anyhow;
use k8s_openapi::api::core::v1::{
    ContainerState, ContainerStateRunning, ContainerStatus as KubeContainerStatus, PodCondition,
};
use krator::ObjectStatus;
use kubelet::pod::state::prelude::*;
use kubelet::pod::Pod;
use log::{debug, info, trace};

use super::failed::Failed;
use super::installing::Installing;
use crate::provider::states::make_status_with_containers_and_condition;
use crate::provider::{PodState, ProviderState};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use k8s_openapi::chrono;
use tokio::time::Duration;

#[derive(Debug, TransitionTo)]
#[transition_to(Failed, Running, Installing)]
pub struct Running {
    pub transition_time: Time,
}

impl Default for Running {
    fn default() -> Self {
        Self {
            transition_time: Time(chrono::offset::Utc::now()),
        }
    }
}

#[async_trait::async_trait]
impl State<PodState> for Running {
    async fn next(
        mut self: Box<Self>,
        provider_state: SharedState<ProviderState>,
        pod_state: &mut PodState,
        _pod: Manifest<Pod>,
    ) -> Transition<PodState> {
        let systemd_manager = {
            let provider_state = provider_state.read().await;
            provider_state.systemd_manager.clone()
        };

        // We loop here indefinitely and "wake up" periodically to check if the service is still
        // up and running
        // Interruption of this loop is triggered externally by the Krustlet code when
        //   - the pod which this state machine refers to gets deleted
        //   - Krustlet shuts down
        loop {
            tokio::time::sleep(Duration::from_secs(10)).await;
            trace!(
                "Checking if service {} is still running.",
                &pod_state.service_name
            );

            // Iterate over all units and check their state
            // if the [`service_units`] Option is a None variant, return a failed state
            // as we need to run something otherwise we are not doing anything
            let systemd_units = match &pod_state.service_units {
                Some(units) => units,
                None => return Transition::Complete(Err(anyhow!(format!("No systemd units found for service [{}], this should not happen, please report a bug for this!", pod_state.service_name)))),
            };

            for unit in systemd_units {
                match systemd_manager.is_running(&unit.get_name()) {
                    Ok(true) => trace!(
                        "Unit [{}] of service [{}] still running ...",
                        &unit.get_name(),
                        pod_state.service_name
                    ),
                    Ok(false) => {
                        info!("Unit [{}] for service [{}] failed unexpectedly, transitioning to failed state.", pod_state.service_name, unit.get_name());
                        return Transition::next(
                            self,
                            Failed {
                                message: "".to_string(),
                            },
                        );
                    }
                    Err(dbus_error) => {
                        info!(
                            "Error querying ActiveState for Unit [{}] of service [{}]: [{}].",
                            pod_state.service_name,
                            unit.get_name(),
                            dbus_error
                        );
                        return Transition::Complete(Err(dbus_error));
                    }
                }
            }
        }
    }

    // test
    async fn status(&self, pod_state: &mut PodState, pod: &Pod) -> anyhow::Result<PodStatus> {
        let state = ContainerState {
            running: Some(ContainerStateRunning { started_at: None }),
            ..Default::default()
        };

        let container = &pod.containers()[0];
        // TODO: Change to support multiple containers
        let container_status = vec![KubeContainerStatus {
            name: container.name().to_string(),
            ready: true,
            started: Some(false),
            state: Some(state),
            ..Default::default()
        }];
        let condition = PodCondition {
            last_probe_time: None,
            last_transition_time: Some(self.transition_time.clone()),
            message: Some(String::from("Service is running")),
            reason: Some(String::from("Running")),
            status: "True".to_string(),
            type_: "Ready".to_string(),
        };
        let status = make_status_with_containers_and_condition(
            Phase::Running,
            "Running",
            container_status,
            vec![],
            vec![condition],
        );
        debug!(
            "Patching status for running servce [{}] with: [{}]",
            pod_state.service_name,
            status.json_patch()
        );
        Ok(status)
    }
}
