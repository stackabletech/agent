use k8s_openapi::api::core::v1::{
    ContainerState, ContainerStateRunning, ContainerStatus as KubeContainerStatus, PodCondition,
};
use kubelet::pod::Pod;
use kubelet::state::prelude::*;
use kubelet::state::{State, Transition};
use log::{debug, trace};

use crate::provider::states::failed::Failed;
use crate::provider::states::installing::Installing;
use crate::provider::states::make_status_with_containers_and_condition;
use crate::provider::PodState;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use k8s_openapi::chrono;

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
        pod_state: &mut PodState,
        _pod: &Pod,
    ) -> Transition<PodState> {
        loop {
            tokio::select! {
                _ = tokio::time::delay_for(std::time::Duration::from_secs(10))  => {
                    trace!("Checking if service {} is still running.", &pod_state.service_name);
                }
            }
            // TODO: We are not watching the service yet, need to subscribe to events and
            // react to those
        }
    }

    async fn json_status(
        &self,
        pod_state: &mut PodState,
        pod: &Pod,
    ) -> anyhow::Result<serde_json::Value> {
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
            pod_state.service_name, status
        );
        Ok(status)
    }
}
