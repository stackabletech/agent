use kubelet::state::prelude::*;
use log::{error, info, warn};

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
            "Pod {} was terminated, stopping service!",
            &pod_state.service_name
        );

        // TODO: We need some additional error handling here, wait for the services to actually
        //  shut down and try to remove the rest of the services if one fails (tbd, do we want that?)
        if let Some(systemd_units) = &pod_state.service_units {
            for unit in &systemd_units.systemd_units {
                info!("Stopping systemd unit [{}]", unit);
                if let Err(stop_error) = pod_state.systemd_manager.stop(&unit.get_name()) {
                    error!(
                        "Error occurred stopping systemd unit [{}]: [{}]",
                        unit.get_name(),
                        stop_error
                    );
                    return Transition::Complete(Err(stop_error));
                }

                // Daemon reload is false here, we'll do that once after all units have been removed
                info!("Removing systemd unit [{}]", &unit);
                if let Err(remove_error) = pod_state
                    .systemd_manager
                    .remove_unit(&unit.get_name(), false)
                {
                    error!(
                        "Error occurred stopping systemd unit [{}]: [{}]",
                        unit, remove_error
                    );
                    return Transition::Complete(Err(remove_error));
                }
            }
        } else {
            warn!("No unit definitions found, not starting anything!");
        }

        info!("Performing daemon-reload");
        return match pod_state.systemd_manager.reload() {
            Ok(()) => Transition::Complete(Ok(())),
            Err(reload_error) => {
                error!("Failed to perform daemon-reload: [{}]", reload_error);
                Transition::Complete(Err(reload_error))
            }
        };
    }

    async fn json_status(
        &self,
        _pod_state: &mut PodState,
        _pod: &Pod,
    ) -> anyhow::Result<serde_json::Value> {
        make_status(Phase::Succeeded, &self.message)
    }
}
