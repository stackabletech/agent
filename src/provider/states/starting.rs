use kubelet::pod::Pod;
use kubelet::state::prelude::*;
use kubelet::state::{State, Transition};

use crate::provider::states::failed::Failed;
use crate::provider::states::running::Running;
use crate::provider::states::setup_failed::SetupFailed;
use crate::provider::PodState;
use log::{error, info, warn};

#[derive(Default, Debug, TransitionTo)]
#[transition_to(Running, Failed, SetupFailed)]
pub struct Starting;

#[async_trait::async_trait]
impl State<PodState> for Starting {
    async fn next(self: Box<Self>, pod_state: &mut PodState, _: &Pod) -> Transition<PodState> {
        if let Some(systemd_units) = &pod_state.service_units {
            for unit in &systemd_units.systemd_units {
                info!("Starting systemd unit [{}]", unit);
                if let Err(start_error) = pod_state.systemd_manager.start(&unit.get_name()) {
                    error!(
                        "Error occurred starting systemd unit [{}]: [{}]",
                        unit.get_name(),
                        start_error
                    );
                    return Transition::Complete(Err(start_error));
                }

                info!("Enabling systed unit [{}]", unit);
                if let Err(enable_error) = pod_state.systemd_manager.enable(&unit.get_name()) {
                    error!(
                        "Error occurred starting systemd unit [{}]: [{}]",
                        unit.get_name(),
                        enable_error
                    );
                    return Transition::Complete(Err(enable_error));
                }
            }
        } else {
            warn!("No unit definitions found, not starting anything!");
        }
        Transition::next(
            self,
            Running {
                ..Default::default()
            },
        )
    }

    async fn json_status(
        &self,
        _pod_state: &mut PodState,
        _pod: &Pod,
    ) -> anyhow::Result<serde_json::Value> {
        make_status(Phase::Pending, &"Starting")
    }
}
