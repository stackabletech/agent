use kubelet::pod::Pod;
use kubelet::state::{State, Transition};
use kubelet::state::prelude::*;

use crate::provider::PodState;
use crate::provider::states::failed::Failed;
use crate::provider::states::stopped::Stopped;

#[derive(Default, Debug, TransitionTo)]
#[transition_to(Stopped, Failed)]
pub struct Stopping;


#[async_trait::async_trait]
impl State<PodState> for Stopping {
    async fn next(self: Box<Self>, pod_state: &mut PodState, _pod: &Pod) -> Transition<PodState> {
        println!("stopping");
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