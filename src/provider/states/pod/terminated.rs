use kubelet::pod::{state::prelude::*, PodKey};
use log::{error, info, warn};

use crate::provider::{PodState, ProviderState};

#[derive(Default, Debug)]
/// The pod object was deleted in Kubernetes
pub struct Terminated {
    pub message: String,
}

#[async_trait::async_trait]
impl State<PodState> for Terminated {
    async fn next(
        self: Box<Self>,
        shared: SharedState<ProviderState>,
        pod_state: &mut PodState,
        pod: Manifest<Pod>,
    ) -> Transition<PodState> {
        info!(
            "Pod {} was terminated, stopping service!",
            &pod_state.service_name
        );

        let pod = pod.latest();
        let pod_key = &PodKey::from(pod);

        let (systemd_manager, pod_handle) = {
            let provider_state = shared.write().await;
            let mut handles = provider_state.handles.write().await;
            (
                provider_state.systemd_manager.clone(),
                handles.remove(&pod_key),
            )
        };

        // TODO: We need some additional error handling here, wait for the services to actually
        //  shut down and try to remove the rest of the services if one fails (tbd, do we want that?)
        if let Some(containers) = pod_handle {
            for container_handle in containers.values() {
                let service_unit = &container_handle.service_unit;

                info!("Stopping systemd unit [{}]", service_unit);
                if let Err(stop_error) = systemd_manager.stop(service_unit) {
                    error!(
                        "Error occurred stopping systemd unit [{}]: [{}]",
                        service_unit, stop_error
                    );
                    return Transition::Complete(Err(stop_error));
                }

                // Daemon reload is false here, we'll do that once after all units have been removed
                info!("Removing systemd unit [{}]", service_unit);
                if let Err(remove_error) = systemd_manager.remove_unit(service_unit, false) {
                    error!(
                        "Error occurred removing systemd unit [{}]: [{}]",
                        service_unit, remove_error
                    );
                    return Transition::Complete(Err(remove_error));
                }
            }
        } else {
            warn!(
                "No unit definitions found, not stopping anything for pod [{}]!",
                pod_state.service_name
            );
        }

        info!("Performing daemon-reload");
        return match systemd_manager.reload() {
            Ok(()) => Transition::Complete(Ok(())),
            Err(reload_error) => {
                error!("Failed to perform daemon-reload: [{}]", reload_error);
                Transition::Complete(Err(reload_error))
            }
        };
    }

    async fn status(&self, _pod_state: &mut PodState, _pod: &Pod) -> anyhow::Result<PodStatus> {
        Ok(make_status(Phase::Succeeded, &self.message))
    }
}
