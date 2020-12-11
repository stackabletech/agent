use anyhow::anyhow;
use kubelet::state::prelude::*;
use log::{debug, error, info};

use crate::provider::PodState;

#[derive(Default, Debug)]
/// The pod object was deleted in Kubernetes
pub struct Terminated {
    pub message: String,
}

#[async_trait::async_trait]
impl State<PodState> for Terminated {
    async fn next(self: Box<Self>, pod_state: &mut PodState, _pod: &Pod) -> Transition<PodState> {
        info!(
            "Pod {} was terminated, stopping process!",
            &pod_state.service_name
        );
        // Obtain a mutable reference to the process handle
        let child = if let Some(testproc) = pod_state.process_handle.as_mut() {
            testproc
        } else {
            return Transition::Complete(Err(anyhow!("Unable to retrieve process handle")));
        };

        return match child.kill() {
            Ok(()) => {
                debug!("Successfully killed process {}", pod_state.service_name);
                Transition::Complete(Ok(()))
            }
            Err(e) => {
                error!(
                    "Failed to stop process with pid {} due to: {:?}",
                    child.id(),
                    e
                );
                Transition::Complete(Err(anyhow::Error::new(e)))
            }
        };
    }

    async fn json_status(
        &self,
        _pod_state: &mut PodState,
        _pod: &Pod,
    ) -> anyhow::Result<serde_json::Value> {
        make_status(Phase::Succeeded, &self.message)
    }
}
