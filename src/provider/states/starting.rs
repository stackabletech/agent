use std::ffi::OsStr;
use std::process::{Command, Stdio};

use kubelet::pod::Pod;
use kubelet::state::prelude::*;
use kubelet::state::{State, Transition};
use log::{debug, error, info, trace};
use tokio::time::Duration;

use crate::provider::states::creating_config::CreatingConfig;
use crate::provider::states::failed::Failed;
use crate::provider::states::running::Running;
use crate::provider::states::setup_failed::SetupFailed;
use crate::provider::PodState;

#[derive(Default, Debug, TransitionTo)]
#[transition_to(Running, Failed, SetupFailed)]
pub struct Starting;

#[async_trait::async_trait]
impl State<PodState> for Starting {
    async fn next(self: Box<Self>, pod_state: &mut PodState, pod: &Pod) -> Transition<PodState> {
        let container = pod.containers()[0].clone();
        let template_data = if let Ok(data) = CreatingConfig::create_render_data(&pod_state) {
            data
        } else {
            error!("Unable to parse directories for command template as UTF8");
            return Transition::next(
                self,
                SetupFailed {
                    message: "DirectoryParseError".to_string(),
                },
            );
        };
        if let Some(mut command) = container.command().clone() {
            // We need to reverse the vec here, because pop works on the wrong "end" of
            // the vec for our purposes
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
                    .map(|s| CreatingConfig::render_config_template(&template_data, s).unwrap())
                    .collect();

                debug!(
                    "Starting command: {:?} with arguments {:?}",
                    binary, os_args
                );

                // Check if environment variables are set on the container - if some are present
                // we render all values as templates to replace configroot, packageroot and logroot
                // directories in case they are referenced in the values
                //
                // If even one of these renderings fails the entire pod will be failed and
                // transitioned to a complete state with the error that occurred.
                // If all renderings work, the vec<(String,String)> is returned as value and used
                // later when starting the process
                // This works because Result implements
                // (FromIterator)[https://doc.rust-lang.org/std/result/enum.Result.html#method.from_iter]
                // which returns a Result that is Ok(..) if none of the internal results contained
                // an Error. If any error occurred, iteration stops on the first error and returns
                // that in the outer result.
                let env_variables = if let Some(vars) = container.env() {
                    debug!(
                        "Got environment vars: {:?} service {}",
                        vars, pod_state.service_name
                    );
                    let render_result = vars
                        .iter()
                        .map(|env_var| {
                            // Replace variables in value
                            CreatingConfig::render_config_template(
                                &template_data,
                                &env_var.value.as_deref().unwrap_or_default(),
                            )
                            .map(|value| (&env_var.name, value))
                        })
                        .collect();

                    // If any single rendering failed, the overall result for the map will have
                    // collected the Err which we can check for here
                    match render_result {
                        Ok(rendered_values) => rendered_values,
                        Err(error) => {
                            error!("Failed to render value for env var due to: {:?}", error);
                            return Transition::Complete(Err(anyhow::Error::from(error)));
                        }
                    }
                } else {
                    // No environment variables present for this container -> empty vec
                    debug!(
                        "No environment vars set for service {}",
                        pod_state.service_name
                    );
                    vec![]
                };
                debug!(
                    "Setting environment for service {} to {:?}",
                    pod_state.service_name, &env_variables
                );

                let start_result = Command::new(binary)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .envs(env_variables)
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
                                error!(
                                    "Process died {} after {} seconds during startup!",
                                    pod_state.service_name, i
                                );
                                return Transition::next(
                                    self,
                                    Failed {
                                        message: "ProcessFailedDuringStartup".to_string(),
                                    },
                                );
                            }
                        }
                        // Store the child handle in the podstate so that later states
                        // can use it
                        pod_state.process_handle = Some(child);
                        return Transition::next(
                            self,
                            Running {
                                ..Default::default()
                            },
                        );
                    }
                    Err(error) => {
                        let error_message = format!("Failed to start process with error {}", error);
                        error!("{}", error_message);
                        return Transition::next(
                            self,
                            Failed {
                                message: "ProcessStartFailed".to_string(),
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
                message: "MissingCommandObject".to_string(),
            },
        );
    }

    async fn json_status(
        &self,
        _pod_state: &mut PodState,
        _pod: &Pod,
    ) -> anyhow::Result<serde_json::Value> {
        make_status(Phase::Pending, &"Starting")
    }
}
