use kubelet::backoff::BackoffStrategy;
use kubelet::pod::state::prelude::*;
use log::info;

use super::downloading::Downloading;
use crate::provider::repository::package::Package;
use crate::provider::{PodState, ProviderState};

#[derive(Debug, TransitionTo)]
#[transition_to(Downloading)]
/// A setup step for the service failed.
pub struct DownloadingBackoff {
    pub package: Package,
}

#[async_trait::async_trait]
impl State<PodState> for DownloadingBackoff {
    async fn next(
        self: Box<Self>,
        _provider_state: SharedState<ProviderState>,
        pod_state: &mut PodState,
        _pod: Manifest<Pod>,
    ) -> Transition<PodState> {
        info!(
            "Backing of before retrying download of package {}",
            self.package
        );
        pod_state.package_download_backoff_strategy.wait().await;
        Transition::next(self, Downloading)
    }

    async fn status(&self, _pod_state: &mut PodState, _pod: &Pod) -> anyhow::Result<PodStatus> {
        Ok(make_status(Phase::Pending, &"DownloadingBackoff"))
    }
}
