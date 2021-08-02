use kubelet::backoff::BackoffStrategy;
use kubelet::pod::state::prelude::*;
use log::info;

use super::creating_config::CreatingConfig;
use crate::provider::{PodState, ProviderState};

#[derive(Debug, TransitionTo)]
#[transition_to(CreatingConfig)]
/// A config map that was specified in the pod has not yet been created in the apiserver, back off
/// until this has been created
/// TODO: make this a watch instead of delay
pub struct WaitingConfigMap {
    pub missing_config_maps: Vec<String>,
}

#[async_trait::async_trait]
impl State<PodState> for WaitingConfigMap {
    async fn next(
        self: Box<Self>,
        _provider_state: SharedState<ProviderState>,
        pod_state: &mut PodState,
        _pod: Manifest<Pod>,
    ) -> Transition<PodState> {
        info!(
            "Delaying execution due to missing configmaps: {:?}",
            &self.missing_config_maps
        );
        pod_state.package_download_backoff_strategy.wait().await;

        Transition::next(
            self,
            CreatingConfig {
                target_directory: None,
            },
        )
    }

    async fn status(&self, _pod_state: &mut PodState, _pod: &Pod) -> anyhow::Result<PodStatus> {
        Ok(make_status(Phase::Pending, "WaitingConfigMap"))
    }
}
