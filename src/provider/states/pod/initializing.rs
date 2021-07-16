use std::net::IpAddr;

use anyhow::Result;
use k8s_openapi::api::core::v1::Pod as KubePod;
use k8s_openapi::api::core::v1::PodStatus as KubePodStatus;
use kube::api::Patch;
use kube::api::PatchParams;
use kube::Api;
use kubelet::pod::state::prelude::*;
use log::trace;
use log::warn;
use serde_json::json;

use super::downloading::Downloading;
use crate::provider::{PodState, ProviderState};

#[derive(Default, Debug, TransitionTo)]
#[transition_to(Downloading)]
pub struct Initializing;

#[async_trait::async_trait]
impl State<PodState> for Initializing {
    async fn next(
        self: Box<Self>,
        shared: SharedState<ProviderState>,
        _pod_state: &mut PodState,
        pod: Manifest<Pod>,
    ) -> Transition<PodState> {
        let (client, server_ip_address) = {
            let provider_state = shared.read().await;
            (
                provider_state.client.clone(),
                provider_state.server_ip_address,
            )
        };

        let pod = pod.latest();

        let api = Api::namespaced(client, pod.namespace());

        match patch_ip_address(&api, pod.name(), server_ip_address).await {
            Ok(_) => trace!(
                "Status of pod [{}] patched with hostIP and podIP [{}]",
                pod.name(),
                server_ip_address
            ),
            Err(error) => warn!(
                "Status of pod [{}] could not be patched with hostIP and podIP: {}",
                pod.name(),
                error
            ),
        }

        Transition::next(self, Downloading)
    }

    async fn status(&self, _pod_state: &mut PodState, _pod: &Pod) -> Result<PodStatus> {
        Ok(make_status(Phase::Pending, "Initializing"))
    }
}

/// Patches the `hostIP` and `podIP` in the pod status.
async fn patch_ip_address(api: &Api<KubePod>, pod_name: &str, ip_address: IpAddr) -> Result<()> {
    let patch = json!({
        "status": Some(KubePodStatus {
            host_ip: Some(ip_address.to_string()),
            pod_ip: Some(ip_address.to_string()),
            ..Default::default()
        })
    });

    api.patch_status(pod_name, &PatchParams::default(), &Patch::Strategic(patch))
        .await?;

    Ok(())
}
