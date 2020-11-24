use std::collections::BTreeMap;
use std::fs;
use std::fs::read_to_string;
use std::path::PathBuf;

use handlebars::Handlebars;
use k8s_openapi::api::core::v1::ConfigMap;
use kube::{Api, Client};
use kubelet::pod::Pod;
use kubelet::state::{State, Transition};
use kubelet::state::prelude::*;
use log::{debug, error, info, trace, warn};

use crate::fail_fatal;
use crate::provider::error::StackableError;
use crate::provider::error::StackableError::PodValidationError;
use crate::provider::PodState;
use crate::provider::states::create_service::CreatingService;
use crate::provider::states::setup_failed::SetupFailed;
use crate::provider::states::waiting_config::WaitingConfigMap;

#[derive(Default, Debug, TransitionTo)]
#[transition_to(CreatingService, SetupFailed, WaitingConfigMap)]
pub struct CreatingConfig {
    pub target_directory: Option<PathBuf>,
}

impl CreatingConfig {
    pub fn render_config_template(
        data: BTreeMap<String, String>,
        template: String,
    ) -> Result<String, StackableError> {
        let mut handlebars = Handlebars::new();
        debug!("Rendering template with context: {:?}", data);

        // register the template. The template string will be verified and compiled.
        handlebars.register_template_string("t1", template)?;

        // Set strict mode, so that we fail with an error if any non-existent fields are accessed
        handlebars.set_strict_mode(true);

        // Render the template with the provided data and return the resulting String
        Ok(handlebars.render("t1", &data)?)
    }

    pub fn create_render_data(pod_state: &PodState) -> BTreeMap<String, String> {
        let mut render_data = BTreeMap::new();
        let directory_name = pod_state.package.get_directory_name();

        if let Ok(package_dir) = &pod_state
            .parcel_directory
            .join(&directory_name)
            .into_os_string()
            .into_string()
        {
            render_data.insert(String::from("packageroot"), String::from(package_dir));
        } else {
            warn!("Unable to parse value for package directory as UTF8")
        }

        if let Ok(conf_dir) = &pod_state
            .config_directory
            .join(&directory_name)
            .into_os_string()
            .into_string()
        {
            render_data.insert(String::from("configroot"), String::from(conf_dir));
        } else {
            warn!("Unable to parse value for config directory as UTF8");
        }
        render_data
    }

    async fn missing_config_maps(&self, client: Client, configmaps: Vec<String>) -> Vec<String> {
        // TODO: distinguish between an actually missing configmap and an error when talking to
        // the apiserver
        let configmaps_api: Api<ConfigMap> = Api::namespaced(client.clone(), "default");
        let mut missing_configmaps = vec![];
        for map in configmaps {
            let result = configmaps_api.get(&map).await;
            match result {
                Ok(configmap) => {}
                Err(e) => {
                    debug!("ConfigMap {} not found due to error {:?}", &map, e);
                    missing_configmaps.push(String::from(map));
                }
            }
        }
        missing_configmaps
    }

    async fn get_config_maps(&self, pod: &Pod) -> Vec<String> {
        let mut get_config_maps = vec![];

        if let Some(volumes) = pod.volumes() {
            for volume in volumes {
                if let Some(config_map) = &volume.config_map {
                    // config map was present, check if a name was set
                    // not sure when it would not be set, but it is a valid possibility, so we need
                    // to handle it - if no name is present, we'll just ignore this map, not sure
                    // how to retrieve it otherwise
                    if let Some(config_map_name) = &config_map.name {
                        debug!("Found reference to config map {}", &config_map_name);
                        get_config_maps.push(String::from(config_map_name));
                    }
                }
            }
        }
        get_config_maps
    }

    async fn retrieve_config_map(
        &self,
        client: Client,
        name: String,
    ) -> Result<ConfigMap, StackableError> {
        let config_maps: Api<ConfigMap> = Api::namespaced(client.clone(), "default");

        Ok(config_maps.get(&name).await?)
    }

    fn apply_config_map(
        &self,
        map: ConfigMap,
        target_directory: PathBuf,
        template_data: &BTreeMap<String, String>,
    ) -> Result<(), StackableError> {
        let config_map_name = map.metadata.name.unwrap_or(String::from("undefined"));
        debug!(
            "applying configmap {} to directory {:?}",
            &config_map_name, target_directory
        );
        if !(&target_directory.is_dir()) {
            info!("creating config directory {:?}", target_directory);
            fs::create_dir_all(&target_directory)?;
        }
        if let Some(data) = map.data {
            debug!("Map contained keys: {:?}", &data.keys());
            for key in data.keys() {
                debug!("found key: {} in configmap {}", key, &config_map_name);
                if let Some(content) = data.get(key) {
                    trace!("content of key: {}", &content);
                    debug!("rendering");
                    let rendered_content = CreatingConfig::render_config_template(
                        template_data.clone(),
                        content.clone(),
                    )?;
                    debug!("done rendering");
                    let target_file = target_directory.join(&key);

                    // TODO: compare existing file with intended state
                    if CreatingConfig::needs_update(&target_file, &rendered_content)? {
                        debug!(
                            "writing content of map entry {} to file {:?}",
                            key, target_file
                        );
                        let write_result = fs::write(target_directory.join(&key), rendered_content);
                        match write_result {
                            Ok(()) => debug!("write of file {:?} successful!", target_file),
                            Err(e) => error!("write of file {:?} failed: {}", target_file, e),
                        }
                    } else {
                        debug!("No update needed for {:?}", target_file);
                    }
                } else {
                    info!("No content found for key {}", key);
                }
            }
        } else {
            debug!("No data found in ConfigMap..");
        }
        Ok(())
    }

    fn needs_update(target_file: &PathBuf, content: &str) -> Result<bool, StackableError> {
        if target_file.is_file() {
            let current_content = read_to_string(target_file)?;
            debug!("Compared config file {:?} with result of", target_file);
            return Ok(current_content.ne(content));
        }
        debug!(
            "Target config file {:?} doesn't exist, no need to compare.",
            target_file
        );
        Ok(true)
    }
}

#[async_trait::async_trait]
impl State<PodState> for CreatingConfig {
    async fn next(
        mut self: Box<Self>,
        pod_state: &mut PodState,
        _pod: &Pod,
    ) -> Transition<PodState> {
        let name = _pod.name();
        let client = pod_state.client.clone();
        let package = pod_state.package.clone();
        let config_directory = pod_state.config_directory.clone();
        self.target_directory = Some(config_directory.join(package.get_directory_name()));
        let target_directory = self.target_directory.clone().unwrap();

        // Check if all required config maps have been created in the api-server
        let referenced_config_maps = self.get_config_maps(_pod).await;
        let missing_config_maps = self
            .missing_config_maps(client.clone(), referenced_config_maps)
            .await;
        if !missing_config_maps.is_empty() {
            // not all configmaps are present
            info!("Missing config maps, waiting..");
            return Transition::next(
                self,
                WaitingConfigMap {
                    missing_config_maps,
                },
            );
        }

        debug!("Entering state \"creating config\" for service {}", name);
        let containers = _pod.containers();
        if containers.len().ne(&1) {
            let e = PodValidationError {
                msg: "Only pods containing exactly one container element are supported!"
                    .to_string(),
            };
            fail_fatal!(e);
        }
        let container = containers[0].clone();

        if let Some(volumes) = _pod.volumes() {
            debug!("Found {} volumes in pod {}", volumes.len(), _pod.name());
            if let Some(mounts) = container.volume_mounts() {
                debug!("Found {} mounts in pod {}", mounts.len(), _pod.name());
                // Got mounts and volumes, we can now decide which ones we need to act upon
                for mount in mounts {
                    for volume in volumes {
                        if mount.name.eq(&volume.name) {
                            let target_dir = target_directory.join(&mount.mount_path);
                            if let Some(config_map) = &volume.config_map {
                                if let Some(map_name) = &config_map.name {
                                    if let Ok(map) = self
                                        .retrieve_config_map(client.clone(), map_name.to_string())
                                        .await
                                    {
                                        debug!("found config map: {:?} - applying", config_map);
                                        self.apply_config_map(
                                            map,
                                            target_dir,
                                            &CreatingConfig::create_render_data(pod_state),
                                        );
                                    }
                                }
                            } else {
                                warn!("Skipping volume {} - it is not a config map", volume.name);
                            }
                        }
                    }
                }
            };
        }
        debug!("Transitioning to service creation");
        Transition::next(self, CreatingService)
    }

    async fn json_status(
        &self,
        _pod_state: &mut PodState,
        _pod: &Pod,
    ) -> anyhow::Result<serde_json::Value> {
        make_status(Phase::Pending, &"status:initializing")
    }
}
