use kubelet::pod::state::prelude::*;
use kubelet::{
    container::ContainerKey,
    pod::{Pod, PodKey},
};
use log::{debug, error, info};

use super::setup_failed::SetupFailed;
use super::starting::Starting;
use crate::provider::systemdmanager::systemdunit::SystemDUnit;
use crate::provider::{PodState, ProviderState};
use std::fs::create_dir_all;

#[derive(Default, Debug, TransitionTo)]
#[transition_to(Starting, SetupFailed)]
pub struct CreatingService;

#[async_trait::async_trait]
impl State<PodState> for CreatingService {
    async fn next(
        self: Box<Self>,
        shared: SharedState<ProviderState>,
        pod_state: &mut PodState,
        pod: Manifest<Pod>,
    ) -> Transition<PodState> {
        let pod = pod.latest();

        let systemd_manager = {
            let provider_state = shared.read().await;
            provider_state.systemd_manager.clone()
        };

        let service_name: &str = pod_state.service_name.as_ref();
        info!(
            "Creating service unit for service {}",
            &pod_state.service_name
        );
        let service_directory = &pod_state.get_service_service_directory();
        if !service_directory.is_dir() {
            debug!(
                "Creating config directory for service [{}]: {:?}",
                pod_state.service_name, service_directory
            );
            if let Err(error) = create_dir_all(service_directory) {
                return Transition::Complete(Err(anyhow::Error::from(error)));
            }
        }

        let user_mode = systemd_manager.is_user_mode();

        // Naming schema
        //  Service name: `namespace-podname`
        //  SystemdUnit: `namespace-podname-containername`
        // TODO: add this to the docs in more detail
        let service_prefix = format!("{}-{}-", pod.namespace(), pod.name());

        // Create a template from those settings that are derived directly from the pod, not
        // from container objects
        let unit_template = match SystemDUnit::new_from_pod(&pod, user_mode) {
            Ok(unit) => unit,
            Err(pod_error) => {
                error!(
                    "Unable to create systemd unit template from pod [{}]: [{}]",
                    service_name, pod_error
                );
                return Transition::Complete(Err(anyhow::Error::from(pod_error)));
            }
        };

        // Each pod can map to multiple systemd units/services as each container will get its own
        // systemd unit file/service.
        // Map every container from the pod object to a systemdunit
        for container in &pod.containers() {
            let unit = match SystemDUnit::new(
                &unit_template,
                &service_prefix,
                &container,
                user_mode,
                pod_state,
            ) {
                Ok(unit) => unit,
                Err(err) => return Transition::Complete(Err(anyhow::Error::from(err))),
            };

            // Create the service
            // As per ADR005 we currently write the unit files directly in the systemd
            // unit directory (by passing None as [unit_file_path]).
            match systemd_manager.create_unit(&unit, None, true, true) {
                Ok(()) => {}
                Err(e) => {
                    // TODO: We need to discuss what to do here, in theory we could have loaded
                    // other services already, do we want to stop those?
                    error!(
                        "Failed to create systemd unit for service [{}]",
                        service_name
                    );
                    return Transition::Complete(Err(e));
                }
            }

            {
                let provider_state = shared.write().await;
                let mut handles = provider_state.handles.write().await;
                handles.insert_container_handle(
                    &PodKey::from(&pod),
                    &ContainerKey::App(String::from(container.name())),
                    &unit.get_name(),
                )
            };

            // Done for now, if the service was created successfully we are happy
            // Starting and enabling comes in a later state after all service have been createddy
        }

        // All services were loaded successfully, otherwise we'd have returned early above
        Transition::next(self, Starting)
    }

    async fn status(&self, _pod_state: &mut PodState, _pod: &Pod) -> anyhow::Result<PodStatus> {
        Ok(make_status(Phase::Pending, &"CreatingService"))
    }
}
