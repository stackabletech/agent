use k8s_openapi::api::core::v1::ContainerStatus as KubeContainerStatus;
use k8s_openapi::api::core::v1::PodCondition as KubePodCondition;
use kubelet::pod::state::prelude::*;
use kubelet::pod::{Phase, Status};

pub(crate) mod creating_config;
pub(crate) mod creating_service;
pub(crate) mod downloading;
pub(crate) mod downloading_backoff;
pub(crate) mod failed;
pub(crate) mod installing;
pub(crate) mod running;
pub(crate) mod setup_failed;
pub(crate) mod starting;
pub(crate) mod terminated;
pub(crate) mod waiting_config_map;

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
    _pod_conditions: Vec<KubePodCondition>,
) -> Status {
    // serde_json::json!(
    //    {
    //        "status": {
    //            "phase": phase,
    //            "reason": reason,
    //            "containerStatuses": container_statuses,
    //            "initContainerStatuses": init_container_statuses,
    //            "conditions": pod_conditions
    //        }
    //    }
    // )

    // TODO (sigi) Use custom Status to serialize the pod conditions.
    make_status_with_containers(phase, reason, container_statuses, init_container_statuses)
}
