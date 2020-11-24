use kubelet::pod::Pod;
use kubelet::state::{State, Transition};
use kubelet::state::prelude::*;

use crate::provider::PodState;
use crate::provider::states::setup_failed::SetupFailed;
use crate::provider::states::starting::Starting;

#[derive(Default, Debug, TransitionTo)]
#[transition_to(Starting, SetupFailed)]
pub struct CreatingService;

#[async_trait::async_trait]
impl State<PodState> for CreatingService {
    async fn next(self: Box<Self>, pod_state: &mut PodState, _pod: &Pod) -> Transition<PodState> {
        println!("creating service");
        Transition::next(self, Starting)
    }

    async fn json_status(
        &self,
        _pod_state: &mut PodState,
        _pod: &Pod,
    ) -> anyhow::Result<serde_json::Value> {
        make_status(Phase::Pending, &"status:initializing")
    }
}