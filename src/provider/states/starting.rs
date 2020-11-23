use kubelet::state::{State, Transition};
use kubelet::pod::Pod;
use kubelet::state::prelude::*;
use crate::provider::PodState;
use crate::provider::states::failed::Failed;
use crate::provider::states::running::Running;

#[derive(Default, Debug, TransitionTo)]
#[transition_to(Running, Failed)]
pub struct Starting;


#[async_trait::async_trait]
impl State<PodState> for Starting {
    async fn next(self: Box<Self>, pod_state: &mut PodState, _pod: &Pod) -> Transition<PodState> {
        println!("starting");
        Transition::next(self, Running)
    }

    async fn json_status(
        &self,
        _pod_state: &mut PodState,
        _pod: &Pod,
    ) -> anyhow::Result<serde_json::Value> {
        make_status(Phase::Pending, &"status:running")
    }
}