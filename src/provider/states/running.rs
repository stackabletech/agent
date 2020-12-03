use k8s_openapi::api::core::v1::{
    ContainerState, ContainerStateRunning, ContainerStatus as KubeContainerStatus,
};
use kubelet::pod::Pod;
use kubelet::state::prelude::*;
use kubelet::state::{State, Transition};
use log::{debug, error, trace};

use crate::provider::states::failed::Failed;
use crate::provider::states::installing::Installing;
use crate::provider::states::stopping::Stopping;
use crate::provider::PodState;

#[derive(Debug, TransitionTo)]
#[transition_to(Stopping, Failed, Running, Installing)]
pub struct Running {}

#[async_trait::async_trait]
impl State<PodState> for Running {
    async fn next(
        mut self: Box<Self>,
        pod_state: &mut PodState,
        _pod: &Pod,
    ) -> Transition<PodState> {
        loop {
            tokio::select! {
                _ = tokio::time::delay_for(std::time::Duration::from_secs(1))  => {
                    trace!("Checking if service {} is still running.", &pod_state.service_name);
                }
            }

            // Obtain a mutable reference to the process handle
            let child = if let Some(testproc) = pod_state.process_handle.as_mut() {
                testproc
            } else {
                return Transition::next(
                    self,
                    Failed {
                        message: "Unable to obtain process handle from podstate!".to_string(),
                    },
                );
            };

            // Check if an exit code is available for the process - if yes, it exited
            match child.try_wait() {
                Ok(None) => debug!(
                    "Service {} is still running with pid {}",
                    &pod_state.service_name,
                    child.id()
                ),
                _ => {
                    error!(
                        "Service {} died unexpectedly, moving to failed state",
                        pod_state.service_name
                    );
                    return Transition::next(
                        self,
                        Failed {
                            message: "ProcessDiedUnexpectedly".to_string(),
                        },
                    );
                }
            }
        }
    }

    async fn json_status(
        &self,
        _pod_state: &mut PodState,
        pod: &Pod,
    ) -> anyhow::Result<serde_json::Value> {
        let state = ContainerState {
            running: Some(ContainerStateRunning { started_at: None }),
            ..Default::default()
        };

        let container = &pod.containers()[0];
        let mut container_status = vec![];
        container_status.push(KubeContainerStatus {
            name: container.name().to_string(),
            ready: true,
            started: Some(false),
            state: Some(state),
            ..Default::default()
        });
        Ok(make_status_with_containers(
            Phase::Running,
            "Running",
            container_status,
            vec![],
        ))
    }
}
