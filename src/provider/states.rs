use k8s_openapi::api::core::v1::ContainerStatus as KubeContainerStatus;
use k8s_openapi::api::core::v1::PodCondition as KubePodCondition;
use kubelet::pod::state::prelude::*;
use kubelet::pod::{Phase, Status};

pub(crate) mod pod;

/// When called in a state's `next` function, exits the state machine
/// returns a fatal error to the kubelet.
#[macro_export]
macro_rules! fail_fatal {
    ($err:ident) => {{
        let aerr = anyhow::Error::from($err);
        log::error!("{:?}", aerr);
        return Transition::Complete(Err(aerr));
    }};
}

/// Create basic Pod status patch with container status and pod conditions
pub fn make_status_with_containers_and_condition(
    phase: Phase,
    reason: &str,
    container_statuses: Vec<KubeContainerStatus>,
    init_container_statuses: Vec<KubeContainerStatus>,
    conditions: Vec<KubePodCondition>,
) -> Status {
    StatusBuilder::new()
        .phase(phase)
        .reason(reason)
        .container_statuses(container_statuses)
        .init_container_statuses(init_container_statuses)
        .conditions(conditions)
        .build()
}
