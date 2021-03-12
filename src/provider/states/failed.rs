use kubelet::pod::state::prelude::*;
use log::{debug, info};

use crate::provider::states::installing::Installing;
use crate::provider::states::starting::Starting;
use crate::provider::{PodState, ProviderState};

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
    async fn next(
        self: Box<Self>,
        _provider_state: SharedState<ProviderState>,
        _pod_state: &mut PodState,
        pod: Manifest<Pod>,
    ) -> Transition<PodState> {
        let pod = pod.latest();

        info!("Process entered failed state");
        if self.restart_enabled(&pod) {
            debug!("Restart policy is set to restart, starting...");
            return Transition::next(self, Starting {});
        } else {
            debug!("Restart is disabled for process.");
        }
        Transition::Complete(Ok(()))
    }

    async fn status(&self, _pod_state: &mut PodState, _pod: &Pod) -> anyhow::Result<PodStatus> {
        Ok(make_status(Phase::Failed, &self.message))
    }
}
