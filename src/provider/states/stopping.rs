use kubelet::pod::Pod;
use kubelet::state::prelude::*;
use kubelet::state::{State, Transition};
use log::info;

use crate::provider::states::failed::Failed;
use crate::provider::states::stopped::Stopped;
use crate::provider::PodState;

#[derive(Default, Debug, TransitionTo)]
#[transition_to(Stopped, Failed)]
pub struct Stopping;

#[async_trait::async_trait]
impl State<PodState> for Stopping {
    async fn next(self: Box<Self>, pod_state: &mut PodState, _pod: &Pod) -> Transition<PodState> {
        if let Some(child) = &pod_state.process_handle {
            let pid = child.id();
            info!(
                "Received stop command for service {}, stopping process with pid {}",
                pod_state.service_name, pid
            );
        }
        Transition::next(self, Stopped)
    }

    async fn json_status(
        &self,
        _pod_state: &mut PodState,
        _pod: &Pod,
    ) -> anyhow::Result<serde_json::Value> {
        make_status(Phase::Pending, &"status:running")
    }
}
