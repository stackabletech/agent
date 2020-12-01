use std::process::Child;

use kubelet::pod::Pod;
use kubelet::state::prelude::*;
use kubelet::state::{State, Transition};
use log::{debug, error, trace};

use crate::provider::states::failed::Failed;
use crate::provider::states::installing::Installing;
use crate::provider::states::stopping::Stopping;
use crate::provider::PodState;
use k8s_openapi::api::core::v1::{
    ContainerState, ContainerStateRunning, ContainerStatus as KubeContainerStatus,
};

#[derive(Debug, TransitionTo)]
#[transition_to(Stopping, Failed, Running, Installing)]
pub struct Running {
    pub process_handle: Option<Child>,
}

#[async_trait::async_trait]
impl State<PodState> for Running {
    async fn next(
        mut self: Box<Self>,
        pod_state: &mut PodState,
        _pod: &Pod,
    ) -> Transition<PodState> {
        debug!("waiting");
        let mut handle = std::mem::replace(&mut self.process_handle, None).unwrap();
        /*while let Ok(_) = timeout(Duration::from_millis(100), changed.notified()).await {
            debug!("drained a waiting notification");
        }*/
        // debug!("done draining");

        loop {
            tokio::select! {
                /*_ = changed.notified() => {
                    debug!("pod changed");
                    break;
                },*/
                _ = tokio::time::delay_for(std::time::Duration::from_secs(1))  => {
                    trace!("Checking if service {} is still running.", &pod_state.service_name);
                }
            }
            match handle.try_wait() {
                Ok(None) => debug!("Service {} is still running", &pod_state.service_name),
                _ => {
                    error!(
                        "Service {} died unexpectedly, moving to failed state",
                        pod_state.service_name
                    );
                    return Transition::next(
                        self,
                        Failed {
                            message: "Process died unexpectedly!".to_string(),
                        },
                    );
                }
            }
        }
    }

    async fn json_status(
        &self,
        _pod_state: &mut PodState,
        _pod: &Pod,
    ) -> anyhow::Result<serde_json::Value> {
        let state = ContainerState {
            running: Some(ContainerStateRunning { started_at: None }),
            ..Default::default()
        };

        let mut container = &_pod.containers()[0];
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
