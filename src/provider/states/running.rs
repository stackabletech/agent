use std::process::Child;

use kubelet::pod::Pod;
use kubelet::state::prelude::*;
use kubelet::state::{State, Transition};
use log::{debug, error, trace};

use crate::provider::states::failed::Failed;
use crate::provider::states::install_package::Installing;
use crate::provider::states::stopping::Stopping;
use crate::provider::PodState;

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
        make_status(Phase::Running, &"status:running")
    }
}
