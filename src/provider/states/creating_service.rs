use kubelet::pod::Pod;
use kubelet::state::prelude::*;
use kubelet::state::{State, Transition};
use log::{debug, error, info};

use crate::provider::states::setup_failed::SetupFailed;
use crate::provider::states::starting::Starting;
use crate::provider::systemdmanager::service::Service;
use crate::provider::PodState;
use std::fs::create_dir_all;

#[derive(Default, Debug, TransitionTo)]
#[transition_to(Starting, SetupFailed)]
pub struct CreatingService;

#[async_trait::async_trait]
impl State<PodState> for CreatingService {
    async fn next(self: Box<Self>, pod_state: &mut PodState, pod: &Pod) -> Transition<PodState> {
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
            match create_dir_all(service_directory) {
                Ok(()) => {}
                Err(error) => return Transition::Complete(Err(anyhow::Error::from(error))),
            }
        }

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

        /*match service.write_unit_files(service_directory) {
            Ok(()) => Transition::next(self, Starting),
            Err(error) => {
                error!(
                    "Failed to write service units for pod [{}], aborting.",
                    pod_state.service_name
                );
                return Transition::Complete(Err(anyhow::Error::from(error)));
            }
        }*/
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
