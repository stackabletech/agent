use kubelet::state::{State, Transition};
use kubelet::pod::Pod;
use kubelet::state::prelude::*;
use crate::provider::PodState;
use crate::provider::states::failed::Failed;
use crate::provider::states::stopping::Stopping;
use crate::provider::states::install_package::Installing;
use kubelet::container::ContainerKey;
use log::{debug, info, warn, error};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

#[derive(Default, Debug, TransitionTo)]
#[transition_to(Stopping, Failed, Running, Installing)]
pub struct Running;


#[async_trait::async_trait]
impl State<PodState> for Running {
    async fn next(self: Box<Self>, pod_state: &mut PodState, _pod: &Pod) -> Transition<PodState> {
        /*while let Ok(_) = timeout(Duration::from_millis(100), changed.notified()).await {
            debug!("drained a waiting notification");
        }
        debug!("done draining");
        */
        loop {
            println!("running");
            tokio::select! {
                _ = tokio::time::delay_for(std::time::Duration::from_secs(10))  => {
                    debug!("timer expired");
                    continue;
                }
            }
        }
        Transition::next(self, Installing{
            download_directory: pod_state.download_directory.clone(),
            parcel_directory: pod_state.parcel_directory.clone(),
            package: pod_state.package.clone()
        })
    }

    async fn json_status(
        &self,
        _pod_state: &mut PodState,
        _pod: &Pod,
    ) -> anyhow::Result<serde_json::Value> {
        make_status(Phase::Running, &"status:running")
    }
}