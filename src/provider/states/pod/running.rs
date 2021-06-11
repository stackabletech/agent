use anyhow::anyhow;
use k8s_openapi::api::core::v1::PodCondition;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use k8s_openapi::chrono;
use krator::ObjectStatus;
use kubelet::{
    container::Status,
    pod::state::prelude::*,
    pod::{Pod, PodKey},
};
use log::{debug, info, trace, warn};
use tokio::time::Duration;

use super::{installing::Installing, starting::Starting, terminated::Terminated};
use crate::provider::{
    kubernetes::{
        accessor::{restart_policy, RestartPolicy},
        status::patch_container_status,
    },
    PodHandle, PodState, ProviderState,
};

#[derive(Debug, TransitionTo)]
#[transition_to(Installing, Starting, Terminated)]
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
        shared: SharedState<ProviderState>,
        pod_state: &mut PodState,
        pod: Manifest<Pod>,
    ) -> Transition<PodState> {
        let pod = pod.latest();
        let pod_key = &PodKey::from(&pod);

        let (client, systemd_manager, pod_handle) = {
            let provider_state = shared.read().await;
            let handles = provider_state.handles.read().await;
            (
                provider_state.client.clone(),
                provider_state.systemd_manager.clone(),
                handles.get(&pod_key).map(PodHandle::to_owned),
            )
        };

        let mut running_containers = match &pod_handle {
                Some(containers) => containers.to_owned(),
                None => return Transition::Complete(Err(anyhow!("No systemd units found for service [{}], this should not happen, please report a bug for this!", pod_state.service_name))),
            };

        let mut container_failed = false;

        // We loop here and "wake up" periodically to check if the service is still
        // up and running
        // Interruption of this loop is triggered externally by the Krustlet code when
        //   - the pod which this state machine refers to gets deleted
        //   - Krustlet shuts down
        while !running_containers.is_empty() && !container_failed {
            tokio::time::sleep(Duration::from_secs(10)).await;
            trace!(
                "Checking if service {} is still running.",
                &pod_state.service_name
            );

            let mut succeeded_containers = Vec::new();
            let mut failed_containers = Vec::new();

            for (container_key, container_handle) in running_containers.iter() {
                let service_unit = &container_handle.service_unit;

                match systemd_manager.is_running(&service_unit).await {
                    Ok(true) => {}
                    Ok(false) => match systemd_manager.failed(&service_unit).await {
                        Ok(true) => failed_containers
                            .push((container_key.to_owned(), container_handle.to_owned())),
                        Ok(false) => succeeded_containers
                            .push((container_key.to_owned(), container_handle.to_owned())),
                        Err(dbus_error) => warn!(
                            "Error querying Failed property for Unit [{}] of service [{}]: [{}]",
                            service_unit, pod_state.service_name, dbus_error
                        ),
                    },
                    Err(dbus_error) => {
                        warn!(
                            "Error querying ActiveState for Unit [{}] of service [{}]: [{}].",
                            service_unit, pod_state.service_name, dbus_error
                        );
                    }
                }
            }

            for (container_key, container_handle) in &succeeded_containers {
                info!(
                    "Unit [{}] for service [{}] terminated successfully.",
                    pod_state.service_name, container_handle.service_unit
                );
                patch_container_status(
                    &client,
                    &pod,
                    container_key,
                    &Status::terminated("Completed", false),
                )
                .await;
                running_containers.remove(container_key);
            }

            for (container_key, container_handle) in &failed_containers {
                info!(
                    "Unit [{}] for service [{}] failed unexpectedly.",
                    pod_state.service_name, container_handle.service_unit
                );
                patch_container_status(
                    &client,
                    &pod,
                    container_key,
                    &Status::terminated("Error", true),
                )
                .await;
                running_containers.remove(container_key);
            }

            for container_handle in running_containers.values() {
                trace!(
                    "Unit [{}] of service [{}] still running ...",
                    container_handle.service_unit,
                    pod_state.service_name
                );
            }

            container_failed = !failed_containers.is_empty();
        }

        if container_failed {
            if restart_policy(&pod) == RestartPolicy::Never {
                Transition::next(self, Terminated { successful: false })
            } else {
                debug!("Restart policy is set to restart, starting...");
                Transition::next(self, Starting {})
            }
        } else {
            Transition::next(self, Terminated { successful: true })
        }
    }

    async fn status(&self, pod_state: &mut PodState, _pod: &Pod) -> anyhow::Result<PodStatus> {
        let condition = PodCondition {
            last_probe_time: None,
            last_transition_time: Some(self.transition_time.clone()),
            message: Some(String::from("Service is running")),
            reason: Some(String::from("Running")),
            status: "True".to_string(),
            type_: "Ready".to_string(),
        };

        let status = StatusBuilder::new()
            .phase(Phase::Running)
            .reason("Running")
            .conditions(vec![condition])
            .build();

        debug!(
            "Patching status for running service [{}] with: [{}]",
            pod_state.service_name,
            status.json_patch()
        );
        Ok(status)
    }
}
