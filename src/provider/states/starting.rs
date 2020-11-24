use std::ffi::OsStr;
use std::process::{Command, Stdio};

use kubelet::pod::Pod;
use kubelet::state::{State, Transition};
use kubelet::state::prelude::*;
use log::{debug, error, info, trace};
use tokio::time::Duration;

use crate::provider::PodState;
use crate::provider::states::create_config::CreatingConfig;
use crate::provider::states::failed::Failed;
use crate::provider::states::running::Running;

#[derive(Default, Debug, TransitionTo)]
#[transition_to(Running, Failed)]
pub struct Starting;

#[async_trait::async_trait]
impl State<PodState> for Starting {
    async fn next(self: Box<Self>, pod_state: &mut PodState, _pod: &Pod) -> Transition<PodState> {
        let container = _pod.containers()[0].clone();
        let template_data = CreatingConfig::create_render_data(&pod_state);
        if let Some(mut command) = container.command().clone() {
            debug!("Reversing {:?}", &command);
            command.reverse();
            debug!("Processing {:?}", &command);
            if let Some(binary) = command.pop() {
                let binary = pod_state
                    .parcel_directory
                    .join(pod_state.package.clone().get_directory_name())
                    .join(binary);

                let binary = OsStr::new(&binary);
                command.reverse();

                let os_args: Vec<String> = command
                    .iter()
                    .map(|s| {
                        CreatingConfig::render_config_template(
                            template_data.clone(),
                            String::from(s),
                        )
                        .unwrap()
                    })
                    .collect();

                debug!(
                    "Starting command: {:?} with arguments {:?}",
                    binary, os_args
                );
                let start_result = Command::new(binary)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .args(&os_args)
                    .spawn();

                match start_result {
                    Ok(mut child) => {
                        info!(
                            "Successfully executed command \"{:?}\" with args {:?}",
                            binary, &os_args
                        );
                        debug!("Waiting if startup fails..");
                        for i in 1..10 {
                            tokio::time::delay_for(Duration::from_secs(1)).await;
                            if let Ok(None) = child.try_wait() {
                                trace!("Process still alive after {} seconds ..", i);
                            } else {
                                error!("Process died after {} seconds during startup!", i);
                                return Transition::next(
                                    self,
                                    Failed {
                                        message: "process failed during startup".to_string(),
                                    },
                                );
                            }
                        }
                        //pod_state.process_handle = Some(child);
                        return Transition::next(
                            self,
                            Running {
                                process_handle: Some(child),
                            },
                        );
                    }
                    Err(error) => {
                        let error_message = format!("Failed to start process with error {}", error);
                        error!("{}", error_message);
                        return Transition::next(
                            self,
                            Failed {
                                message: error_message,
                            },
                        );
                    }
                }
            }
        }
        error!("No command found, not starting anything..");
        return Transition::next(
            self,
            Failed {
                message: "no command object present, failing process".to_string(),
            },
        );
    }

    async fn json_status(
        &self,
        _pod_state: &mut PodState,
        _pod: &Pod,
    ) -> anyhow::Result<serde_json::Value> {
        make_status(Phase::Pending, &"status:running")
    }
}
