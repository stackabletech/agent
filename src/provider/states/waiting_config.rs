use kubelet::backoff::BackoffStrategy;
use kubelet::state::prelude::*;
use log::{debug, error, info};

use crate::provider::PodState;
use crate::provider::repository::package::Package;
use crate::provider::states::create_config::CreatingConfig;
use crate::provider::states::download_package::Downloading;
use crate::provider::states::install_package::Installing;

#[derive(Debug, TransitionTo)]
#[transition_to(CreatingConfig)]
/// The Pod failed to run.
// If we manually implement, we can allow for arguments.
pub struct WaitingConfigMap {
    pub missing_config_maps: Vec<String> ,
}

#[async_trait::async_trait]
impl State<PodState> for WaitingConfigMap {
    async fn next(self: Box<Self>, pod_state: &mut PodState, _pod: &Pod) -> Transition<PodState> {
        info!("Delaying execution due to missing configmaps: {:?}", &self.missing_config_maps);
        pod_state.package_download_backoff_strategy.wait().await;

        Transition::next(self, CreatingConfig { target_directory: None })
    }

    async fn json_status(
        &self,
        _pod_state: &mut PodState,
        _pod: &Pod,
    ) -> anyhow::Result<serde_json::Value> {
        make_status(Phase::Pending, &"status:running")
    }
}
