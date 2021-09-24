//! Initial cleanup
//!
//! On startup the systemd units in the `system-stackable` slice are compared to the pods assigned
//! to this node. If a systemd unit is as expected then it is kept and the Stackable Agent will
//! take ownership again in the `Starting` stage.  If there is no corresponding pod or the systemd
//! unit differs from the pod specification then it is removed and the Stackable Agent will create
//! a new systemd unit in the `CreatingService` stage.
//!
//! The cleanup stage is implemented as part of the [`StackableProvider`] because the expected
//! content of a systemd unit file can only be determined with the directories configured in the
//! provider.
//!
//! The cleanup code resides in a separate module because the amount of code justifies it and the
//! log output is more meaningful. It makes it clearer whether a systemd unit is removed in the
//! cleanup stage or in the normal process.
use std::collections::HashMap;

use anyhow::Context;
use k8s_openapi::api::core::v1::Pod as KubePod;
use kube::api::{ListParams, Meta, ObjectList};
use kube::Api;
use kubelet::pod::Pod;
use kubelet::provider::Provider;
use log::{debug, error, info, warn};
use tokio::fs::{read_to_string, remove_file};

use super::systemdmanager::systemdunit::SystemDUnit;
use super::systemdmanager::systemdunit::STACKABLE_SLICE;
use super::StackableProvider;

impl StackableProvider {
    /// Removes systemd units without corresponding pods.
    ///
    /// The systemd units in the `system-stackable` slice are compared with the pods assigned to
    /// this node and all units without corresponding pods or which differ from the pod
    /// specifications are removed.
    pub async fn cleanup(&self, node_name: &str) {
        let systemd_manager = &self.shared.systemd_manager;

        if let Err(error) = systemd_manager.reload().await {
            error!(
                "Skipping the cleanup stage because the systemd daemon reload failed. {}",
                error
            );
            return;
        }

        let units_in_slice = match systemd_manager.slice_content(STACKABLE_SLICE).await {
            Ok(units_in_slice) => units_in_slice,
            Err(error) => {
                debug!(
                    "Skipping the cleanup stage because no systemd units were found in the slice \
                    [{}]. {}",
                    STACKABLE_SLICE, error
                );
                return;
            }
        };

        let pods = match self.assigned_pods(node_name).await {
            Ok(pods) => pods.items,
            Err(error) => {
                error!(
                    "The assigned pods could not be retrieved. All systemd units in the slice [{}] \
                    will be removed. {}",
                    STACKABLE_SLICE, error
                );
                Vec::new()
            }
        };

        let mut units_from_pods = HashMap::new();
        for pod in pods {
            let pod_terminating = pod.metadata.deletion_timestamp.is_some();

            match self.units_from_pod(&pod).await {
                Ok(units) => {
                    for (unit_name, content) in units {
                        units_from_pods.insert(unit_name, (content, pod_terminating));
                    }
                }
                Err(error) => warn!(
                    "Systemd units could not be generated for pod [{}/{}]. {}",
                    pod.namespace().unwrap_or_else(|| String::from("default")),
                    pod.name(),
                    error
                ),
            }
        }

        for unit_name in &units_in_slice {
            match units_from_pods.get(unit_name) {
                Some((expected_content, pod_terminating)) => {
                    match self.unit_file_content(unit_name).await {
                        Ok(Some(content)) if &content == expected_content && !pod_terminating => {
                            info!(
                                "The systemd unit [{}] will be kept because a corresponding pod \
                                exists.",
                                unit_name
                            );
                        }
                        Ok(Some(_)) if *pod_terminating => {
                            info!(
                                "The systemd unit [{}] will be removed because the corresponding \
                                pod is terminating.",
                                unit_name
                            );
                            self.remove_unit(unit_name).await;
                        }
                        Ok(Some(content)) => {
                            info!(
                                "The systemd unit [{}] will be removed because it differs from the \
                                corresponding pod specification.\n\
                                expected content:\n\
                                {}\n\n\
                                actual content:\n\
                                {}",
                                unit_name, expected_content, content
                            );
                            self.remove_unit(unit_name).await;
                        }
                        Ok(None) => {
                            info!(
                                "The systemd unit [{}] will be removed because its file path could \
                                not be determined.",
                                unit_name
                            );
                            self.remove_unit(unit_name).await;
                        }
                        Err(error) => {
                            warn!(
                                "The systemd unit [{}] will be removed because the file content \
                                could not be retrieved. {}",
                                unit_name, error
                            );
                            self.remove_unit(unit_name).await;
                        }
                    }
                }
                None => {
                    info!(
                        "The systemd unit [{}] will be removed because no corresponding pod \
                        exists.",
                        unit_name
                    );
                    self.remove_unit(unit_name).await;
                }
            };
        }
    }

    /// Returns a list of all pods assigned to the given node.
    async fn assigned_pods(&self, node_name: &str) -> anyhow::Result<ObjectList<KubePod>> {
        let client = &self.shared.client;

        let api: Api<KubePod> = Api::all(client.to_owned());
        let lp = ListParams::default().fields(&format!("spec.nodeName={}", node_name));
        api.list(&lp).await.with_context(|| {
            format!(
                "The pods assigned to this node (nodeName = [{}]) could not be retrieved.",
                node_name
            )
        })
    }

    /// Creates the systemd unit files for the given pod in memory.
    ///
    /// A mapping from systemd unit file names to the file content is returned.
    async fn units_from_pod(&self, kubepod: &KubePod) -> anyhow::Result<HashMap<String, String>> {
        let systemd_manager = &self.shared.systemd_manager;

        let mut units = HashMap::new();
        let pod = Pod::from(kubepod.to_owned());
        let pod_state = self.initialize_pod_state(&pod).await?;

        for container in pod.containers() {
            let unit = SystemDUnit::new(
                systemd_manager.is_user_mode(),
                &pod_state,
                &self.shared.kubeconfig_path,
                &pod,
                &container,
            )?;
            units.insert(unit.get_name(), unit.get_unit_file_content());
        }

        Ok(units)
    }

    /// Returns the content of the given systemd unit file.
    async fn unit_file_content(&self, unit_name: &str) -> anyhow::Result<Option<String>> {
        let systemd_manager = &self.shared.systemd_manager;

        let file_path_result = systemd_manager
            .fragment_path(unit_name)
            .await
            .with_context(|| {
                format!(
                    "The file path of the unit [{}] could not be determined.",
                    unit_name
                )
            });

        match file_path_result {
            Ok(Some(file_path)) => {
                let file_content = read_to_string(&file_path)
                    .await
                    .with_context(|| format!("The file [{}] could not be read.", file_path))?;
                Ok(Some(file_content))
            }
            Ok(None) => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Stops, disables and removes the given systemd unit.
    async fn remove_unit(&self, unit_name: &str) {
        let systemd_manager = &self.shared.systemd_manager;

        if let Err(error) = systemd_manager.stop(unit_name).await {
            warn!("{}", error);
        }
        if let Err(error) = systemd_manager.disable(unit_name).await {
            warn!("{}", error);
        }
        if let Ok(Some(file_path)) = systemd_manager.fragment_path(unit_name).await {
            debug!("Removing file [{}].", file_path);
            if let Err(error) = remove_file(file_path).await {
                warn!("{}", error);
            }
        }
    }
}
