use kubelet::pod::{state::prelude::*, PodKey};
use log::{debug, info, warn};

use crate::provider::{PodState, ProviderState};

#[derive(Default, Debug)]
/// The pod object was deleted in Kubernetes
pub struct Terminated {
    pub successful: bool,
}

#[async_trait::async_trait]
impl State<PodState> for Terminated {
    async fn next(
        self: Box<Self>,
        shared: SharedState<ProviderState>,
        pod_state: &mut PodState,
        pod: Manifest<Pod>,
    ) -> Transition<PodState> {
        info!("Pod {} was terminated", &pod_state.service_name);

        let pod = pod.latest();
        let pod_key = &PodKey::from(pod);

        let (systemd_manager, pod_handle) = {
            let provider_state = shared.write().await;
            let mut handles = provider_state.handles.write().await;
            (
                provider_state.systemd_manager.clone(),
                handles.remove(pod_key),
            )
        };

        // TODO: We need some additional error handling here, wait for the services to actually
        //  shut down and try to remove the rest of the services if one fails (tbd, do we want that?)
        if let Some(containers) = pod_handle {
            for container_handle in containers.values() {
                let service_unit = &container_handle.service_unit;

                debug!("Stopping systemd unit [{}]", service_unit);
                if let Err(stop_error) = systemd_manager.stop(service_unit).await {
                    warn!(
                        "Error occurred stopping systemd unit [{}]: [{}]",
                        service_unit, stop_error
                    );
                    return Transition::Complete(Err(stop_error));
                }

                // Daemon reload is false here, we'll do that once after all units have been removed
                debug!("Removing systemd unit [{}]", service_unit);
                if let Err(remove_error) = systemd_manager.remove_unit(service_unit, false).await {
                    warn!(
                        "Error occurred removing systemd unit [{}]: [{}]",
                        service_unit, remove_error
                    );
                    return Transition::Complete(Err(remove_error));
                }
            }

            debug!("Performing daemon-reload");
            if let Err(reload_error) = systemd_manager.reload().await {
                warn!("Failed to perform daemon-reload: [{}]", reload_error);
                return Transition::Complete(Err(reload_error));
            };
        } else {
            debug!("Pod [{}] was already terminated", pod_state.service_name);
        }

        Transition::Complete(Ok(()))
    }

    async fn status(&self, _pod_state: &mut PodState, pod: &Pod) -> anyhow::Result<PodStatus> {
        let phase = pod
            .as_kube_pod()
            .status
            .as_ref()
            .and_then(|status| status.phase.as_ref())
            .map(String::as_ref);

        let already_terminated = phase == Some("Succeeded") || phase == Some("Failed");

        let status = if already_terminated {
            Default::default() // no changes to the current status
        } else if self.successful {
            make_status(Phase::Succeeded, "Completed")
        } else {
            make_status(Phase::Failed, "Error")
        };

        Ok(status)
    }
}
