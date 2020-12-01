use kubelet::state::prelude::*;
use log::{error, info};

use crate::provider::states::downloading::Downloading;
use crate::provider::PodState;

#[derive(Default, Debug, TransitionTo)]
#[transition_to(Downloading)]
/// A setup step for the service failed, this can be one of the following:
/// - Download Package
/// - Install Package
/// - Create Config
/// - Create Service
pub struct SetupFailed {
    pub message: String,
}

#[async_trait::async_trait]
impl State<PodState> for SetupFailed {
    async fn next(self: Box<Self>, _pod_state: &mut PodState, pod: &Pod) -> Transition<PodState> {
        error!(
            "setup failed for pod {} due to: {}",
            pod.name(),
            self.message
        );
        info!("Waiting for {} seconds before retrying..", 10);
        // TODO: make this configurable
        tokio::time::delay_for(std::time::Duration::from_secs(10)).await;
        Transition::next(self, Downloading)
    }

    async fn json_status(
        &self,
        _pod_state: &mut PodState,
        _pod: &Pod,
    ) -> anyhow::Result<serde_json::Value> {
        make_status(Phase::Pending, "SetupFailed")
    }
}
