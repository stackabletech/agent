use kubelet::state::prelude::*;
use log::{error, info};

use crate::provider::PodState;
use crate::provider::states::download_package::Downloading;

#[derive(Default, Debug, TransitionTo)]
#[transition_to(Downloading)]
/// The Pod failed to run.
// If we manually implement, we can allow for arguments.
pub struct SetupFailed {
    pub message: String,
}

#[async_trait::async_trait]
impl State<PodState> for SetupFailed {
    async fn next(self: Box<Self>, pod_state: &mut PodState, _pod: &Pod) -> Transition<PodState> {
        error!("setup failed for pod {} due to: {}", _pod.name(), self.message);
        info!("Waiting for {} seconds before retrying..", 10);
        tokio::time::delay_for(std::time::Duration::from_secs(10)).await;
        Transition::next(self, Downloading)
    }

    async fn json_status(
        &self,
        _pod_state: &mut PodState,
        _pod: &Pod,
    ) -> anyhow::Result<serde_json::Value> {
        make_status(Phase::Pending, &self.message)
    }
}
