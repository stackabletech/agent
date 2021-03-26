use kubelet::pod::state::prelude::*;
use log::{error, info};

use super::downloading::Downloading;
use crate::provider::{PodState, ProviderState};

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
    async fn next(
        self: Box<Self>,
        _provider_state: SharedState<ProviderState>,
        _pod_state: &mut PodState,
        pod: Manifest<Pod>,
    ) -> Transition<PodState> {
        let pod = pod.latest();

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

    async fn status(&self, _pod_state: &mut PodState, _pod: &Pod) -> anyhow::Result<PodStatus> {
        Ok(make_status(Phase::Pending, "SetupFailed"))
    }
}
