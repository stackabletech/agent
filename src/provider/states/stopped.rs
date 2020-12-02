use kubelet::pod::Pod;
use kubelet::state::prelude::*;
use kubelet::state::{State, Transition};
use log::info;
use tokio::time::Duration;

use crate::provider::states::starting::Starting;
use crate::provider::PodState;

#[derive(Default, Debug, TransitionTo)]
#[transition_to(Starting)]
pub struct Stopped;

#[async_trait::async_trait]
impl State<PodState> for Stopped {
    async fn next(self: Box<Self>, pod_state: &mut PodState, _pod: &Pod) -> Transition<PodState> {
        let delay = Duration::from_secs(2);
        info!(
            "Service {} stopped, waiting {} seconds before restart.",
            pod_state.service_name,
            delay.as_secs()
        );
        tokio::time::delay_for(delay).await;
        Transition::next(self, Starting)
    }

    async fn json_status(
        &self,
        _pod_state: &mut PodState,
        _pod: &Pod,
    ) -> anyhow::Result<serde_json::Value> {
        make_status(Phase::Pending, &"Stopped")
    }
}
