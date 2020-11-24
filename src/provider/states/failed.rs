use kubelet::state::prelude::*;
use log::{debug, error, info, trace, warn};

use crate::provider::PodState;
use crate::provider::states::install_package::Installing;
use crate::provider::states::starting::Starting;

#[derive(Default, Debug, TransitionTo)]
#[transition_to(Starting, Installing)]
/// The Pod failed to run.
// If we manually implement, we can allow for arguments.
pub struct Failed {
    pub message: String,
}

impl Failed {
    fn restart_enabled(&self, pod : &Pod) -> bool {
        if let Some(pod_spec) =  &pod.as_kube_pod().spec {
            if let Some(restart_policy) = &pod_spec.restart_policy {
                return restart_policy.eq("Always");
            }
        }
        false
    }
}

#[async_trait::async_trait]
impl State<PodState> for Failed {
    async fn next(self: Box<Self>, pod_state: &mut PodState, _pod: &Pod) -> Transition<PodState> {
        info!("Process entered failed state");
        if self.restart_enabled(_pod) {
            debug!("Restart poliy is set to restart, starting...");
            return Transition::next(self, Starting {});
        } else {
            debug!("Restart is disabled for process.");
        }
        //tokio::time::delay_for(std::time::Duration::from_secs(2)).await;
       // T//ransition::next(self, Installing{
        //    download_directory: pod_state.download_directory.clone(),
         //   parcel_directory: pod_state.parcel_directory.clone(),
         //   package: pod_state.package.clone()
        //})
        Transition::Complete(Ok(()))
    }

    async fn json_status(
        &self,
        _pod_state: &mut PodState,
        _pod: &Pod,
    ) -> anyhow::Result<serde_json::Value> {
        make_status(Phase::Failed, &self.message)
    }
}
