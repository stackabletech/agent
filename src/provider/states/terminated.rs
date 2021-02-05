use kubelet::state::prelude::*;
use log::info;

use crate::provider::systemdmanager::manager::UnitTypes;
use crate::provider::PodState;

#[derive(Default, Debug)]
/// The pod object was deleted in Kubernetes
pub struct Terminated {
    pub message: String,
}

#[async_trait::async_trait]
impl State<PodState> for Terminated {
    async fn next(self: Box<Self>, pod_state: &mut PodState, _pod: &Pod) -> Transition<PodState> {
        info!(
            "Pod {} was terminated, stopping process!",
            &pod_state.service_name
        );

        if let Err(e) = pod_state.systemd_manager.stop(&pod_state.service_name) {
            return Transition::Complete(Err(e));
        }
        match pod_state
            .systemd_manager
            .unload(&pod_state.service_name, UnitTypes::Service)
        {
            Ok(()) => Transition::Complete(Ok(())),
            Err(e) => Transition::Complete(Err(e)),
        }
    }

    async fn json_status(
        &self,
        _pod_state: &mut PodState,
        _pod: &Pod,
    ) -> anyhow::Result<serde_json::Value> {
        make_status(Phase::Succeeded, &self.message)
    }
}
