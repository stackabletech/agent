use kubelet::pod::Pod;
use kubelet::state::prelude::*;
use kubelet::state::{State, Transition};
use log::{debug, error, info};

use crate::provider::states::setup_failed::SetupFailed;
use crate::provider::states::starting::Starting;
use crate::provider::systemdmanager::service::Service;
use crate::provider::PodState;
use anyhow::anyhow;
use std::fs::create_dir_all;
use std::path::PathBuf;

#[derive(Default, Debug, TransitionTo)]
#[transition_to(Starting, SetupFailed)]
pub struct CreatingService;

#[async_trait::async_trait]
impl State<PodState> for CreatingService {
    async fn next(self: Box<Self>, pod_state: &mut PodState, pod: &Pod) -> Transition<PodState> {
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

        // Naming schema
        //  Service name: namespace-podname
        //  SystemdUnit: namespace-podname-containername
        // TODO: add this to the docs in more detail
        // Create service containing all systemd units from pod spec
        let service = match Service::new(pod, pod_state) {
            Ok(new_service) => new_service,
            Err(error) => {
                error!(
                    "Failed to create service units from pod [{}], aborting.",
                    pod_state.service_name
                );
                return Transition::Complete(Err(anyhow::Error::from(error)));
            }
        };

        // Each pod can map to multiple systemd units/services as each container will get its own
        // systemd unit file/service.
        // This will iterate over all of them, write the service files to disk and link
        // the service to systemd.
        for unit in &service.systemd_units {
            let target_file = match service_directory
                .join(service_name)
                .into_os_string()
                .into_string()
            {
                Ok(path) => path,
                Err(_) => {
                    // TODO: output proper error message with information
                    return Transition::Complete(Err(anyhow!(
                        "Failed to convert path for service unit file [{}]",
                        service_name
                    )));
                }
            };

            //Some(PathBuf::from(service_directory)),

            // Create the service
            match pod_state
                .systemd_manager
                .create_unit(&unit, None, true, true)
            {
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
            // Done for now, if the service was created successfully we are happy
            // Starting and enabling comes in a later state after all service have been createddy
        }
        pod_state.service_units = Some(service);

        // All services were loaded successfully, otherwise we'd have returned early above
        Transition::next(self, Starting)
    }

    async fn json_status(
        &self,
        _pod_state: &mut PodState,
        _pod: &Pod,
    ) -> anyhow::Result<serde_json::Value> {
        make_status(Phase::Pending, &"CreatingService")
    }
}
