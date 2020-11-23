use kubelet::state::prelude::*;


use crate::provider::PodState;
use crate::provider::states::install_package::Installing;

#[derive(Default, Debug, TransitionTo)]
#[transition_to(Installing)]
/// The Pod failed to run.
// If we manually implement, we can allow for arguments.
pub struct Failed {
    pub message: String,
}

#[async_trait::async_trait]
impl State<PodState> for Failed {
    async fn next(self: Box<Self>, pod_state: &mut PodState, _pod: &Pod) -> Transition<PodState> {
        println!("failed");
        Transition::next(self, Installing{
            download_directory: pod_state.download_directory.clone(),
            parcel_directory: pod_state.parcel_directory.clone(),
            package: pod_state.package.clone()
        })
    }

    async fn json_status(
        &self,
        _pod_state: &mut PodState,
        _pod: &Pod,
    ) -> anyhow::Result<serde_json::Value> {
        make_status(Phase::Failed, &self.message)
    }
}
