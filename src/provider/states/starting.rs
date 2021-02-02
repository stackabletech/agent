use kubelet::pod::Pod;
use kubelet::state::prelude::*;
use kubelet::state::{State, Transition};

use crate::provider::states::failed::Failed;
use crate::provider::states::running::Running;
use crate::provider::states::setup_failed::SetupFailed;
use crate::provider::PodState;

#[derive(Default, Debug, TransitionTo)]
#[transition_to(Running, Failed, SetupFailed)]
pub struct Starting;

#[async_trait::async_trait]
impl State<PodState> for Starting {
    async fn next(self: Box<Self>, pod_state: &mut PodState, _: &Pod) -> Transition<PodState> {
        return match pod_state.systemd_manager.start(&pod_state.service_name) {
            Ok(()) => Transition::next(
                self,
                Running {
                    ..Default::default()
                },
            ),
            Err(e) => Transition::Complete(Err(e)),
        };
    }

    async fn json_status(
        &self,
        _pod_state: &mut PodState,
        _pod: &Pod,
    ) -> anyhow::Result<serde_json::Value> {
        make_status(Phase::Pending, &"Starting")
    }
}
