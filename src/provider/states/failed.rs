use kubelet::state::prelude::*;
use log::{debug, info};

use crate::provider::states::installing::Installing;
use crate::provider::states::starting::Starting;
use crate::provider::PodState;

#[derive(Default, Debug, TransitionTo)]
#[transition_to(Starting, Installing)]
pub struct Failed {
    pub message: String,
}

impl Failed {
    fn restart_enabled(&self, pod: &Pod) -> bool {
        if let Some(pod_spec) = &pod.as_kube_pod().spec {
            if let Some(restart_policy) = &pod_spec.restart_policy {
                return restart_policy.eq("Always");
            }
        }
        false
    }
}

#[async_trait::async_trait]
impl State<PodState> for Failed {
    async fn next(self: Box<Self>, _pod_state: &mut PodState, _pod: &Pod) -> Transition<PodState> {
        info!("Process entered failed state");
        if self.restart_enabled(_pod) {
            debug!("Restart policy is set to restart, starting...");
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
