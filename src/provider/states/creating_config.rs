use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::fs::read_to_string;
use std::path::PathBuf;

use handlebars::Handlebars;
use k8s_openapi::api::core::v1::ConfigMap;
use kube::{Api, Client};
use kubelet::pod::Pod;
use kubelet::state::prelude::*;
use kubelet::state::{State, Transition};
use log::{debug, error, info, trace, warn};

use crate::fail_fatal;
use crate::provider::error::StackableError;
use crate::provider::error::StackableError::{
    ConfigFileWriteError, DirectoryParseError, MissingConfigMapsError, PodValidationError,
    RuntimeError,
};
use crate::provider::states::creating_service::CreatingService;
use crate::provider::states::setup_failed::SetupFailed;
use crate::provider::states::waiting_config_map::WaitingConfigMap;
use crate::provider::PodState;
use kube::error::ErrorResponse;

#[derive(Default, Debug, TransitionTo)]
#[transition_to(CreatingService, SetupFailed, WaitingConfigMap)]
pub struct CreatingConfig {
    pub target_directory: Option<PathBuf>,
}

impl CreatingConfig {
    pub fn render_config_template(
        data: &BTreeMap<String, String>,
        template: &str,
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

    pub fn create_render_data(
        pod_state: &PodState,
    ) -> Result<BTreeMap<String, String>, StackableError> {
        let mut render_data = BTreeMap::new();

        render_data.insert(
            String::from("packageroot"),
            CreatingConfig::pathbuf_to_string(
                "service package directory",
                pod_state.get_service_package_directory(),
            )?,
        );
        render_data.insert(
            String::from("configroot"),
            CreatingConfig::pathbuf_to_string(
                "service config directory",
                pod_state.get_service_config_directory(),
            )?,
        );
        render_data.insert(
            String::from("packageroot"),
            CreatingConfig::pathbuf_to_string(
                "service log directory",
                pod_state.get_service_log_directory(),
            )?,
        );

        // Return all template data
        Ok(render_data)
    }

    // Public for testing
    pub fn pathbuf_to_string(target_field: &str, path: PathBuf) -> Result<String, StackableError> {
        let path_as_string = path.into_os_string().into_string();
        match path_as_string {
            Ok(valid_string) => Ok(valid_string),
            Err(non_utf8) => Err(DirectoryParseError {
                target: target_field.to_string(),
                original: non_utf8,
            }),
        }
    }

    async fn retrieve_config_maps(
        client: Client,
        ns: &str,
        configmaps: Vec<String>,
    ) -> Result<HashMap<String, ConfigMap>, StackableError> {
        // TODO: distinguish between an actually missing configmap and an error when talking to
        // the apiserver
        let configmaps_api: Api<ConfigMap> = Api::namespaced(client.clone(), ns);
        let mut missing_configmaps = vec![];
        let mut found_configmaps = HashMap::new();
        for map in configmaps {
            match configmaps_api.get(&map).await {
                Ok(config_map) => {
                    if let Some(map_name) = &config_map.metadata.name {
                        found_configmaps.insert(String::from(map_name), config_map);
                    } else {
                        warn!("Got config map {} with no name in metadata, this should never have happened!", map);
                        missing_configmaps.push(map);
                    }
                }
                Err(kube::error::Error::Api(ErrorResponse { reason, .. }))
                    if reason == "NotFound" =>
                {
                    // ConfigMap was not created, add it to the list of missing config maps
                    debug!("ConfigMap {} not found", &map);
                    missing_configmaps.push(map);
                }
                Err(e) => {
                    // An error occurred when communicating with the api server
                    // return immediately
                    debug!("Unable to retrieve config maps due to {:?}", e);
                    return Err(StackableError::from(e));
                }
            }
        }
        if missing_configmaps.is_empty() {
            return Ok(found_configmaps);
        }
        Err(MissingConfigMapsError {
            missing_config_maps: missing_configmaps,
        })
    }

    async fn get_config_maps(pod: &Pod) -> Vec<String> {
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

    ///
    ///
    fn apply_config_map(
        map: &ConfigMap,
        target_directory: &PathBuf,
        template_data: &BTreeMap<String, String>,
    ) -> Result<(), StackableError> {
        if map.metadata.name.is_none() {
            return Err(RuntimeError {
                msg: String::from(
                    "Found ConfigMap with no Name set, this should never have happened",
                ),
            });
        }
        let map = map.clone();
        let config_map_name = &map.metadata.name.expect("Got object with no name from K8s, even though we checked for this one line ago - something went seriously wrong!");
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
                    let rendered_content =
                        CreatingConfig::render_config_template(template_data, content)?;
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
                            Err(e) => {
                                error!("write of file {:?} failed: {}", target_file, e);
                                return Err(ConfigFileWriteError {
                                    target_file: target_file.to_str().unwrap().to_string(),
                                    config_map: config_map_name.clone(),
                                });
                            }
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
        pod: &Pod,
    ) -> Transition<PodState> {
        // TODO: this entire function needs to be heavily refactored
        let name = pod.name();
        let client = pod_state.client.clone();

        // Check size of containers array, we currently only allow one container to be present, this
        // might change in the future
        debug!(
            "Found all relevant config maps for service for service {}, writing config files.",
            name
        );
        let container = if pod.containers().len().ne(&1) {
            let e = PodValidationError {
                msg: "Only pods containing exactly one container element are supported!"
                    .to_string(),
            };
            fail_fatal!(e);
        } else {
            pod.containers().get(0).unwrap().clone()
        };

        // Check if the container has mounts defined
        let mounts = if let Some(mounts) = container.volume_mounts() {
            // At least one mount is defined which is fine for now
            mounts
        } else {
            // No mount defined, nothing to do for us
            info!(
                "No mounts defined for service {} - skipping create config step",
                pod_state.service_name
            );
            return Transition::next(self, CreatingService);
        };

        // Check if there are volumes defined for every mount
        let volume_mounts = if let Some(volumes) = pod.volumes() {
            debug!("Found {} volumes in pod {}", volumes.len(), name);
            let mut result = HashMap::new();
            for mount in mounts {
                for volume in volumes {
                    if mount.name.eq(&volume.name) {
                        // This mount references this volume, check if it is a config map volume
                        if let Some(map) = volume.config_map.clone() {
                            let map_name = map.name.unwrap().clone();
                            result.insert(mount.mount_path.clone(), map_name);
                        }
                    }
                }
            }
            result
        } else {
            warn!(
                "No volumes found in service {}, but it had mounts defined. This is most probably an error that should have been caught by Kubernetes, but we'll try our best to continue!",
                pod_state.service_name
            );
            return Transition::next(self, CreatingService);
        };

        // We now have a map of directories to volumes and need to check if all config maps have
        // been created in the api server

        // Retrieve all config map names that are referenced in the pods volume mounts
        // TODO: refactor this to use the map created above
        let referenced_config_maps = CreatingConfig::get_config_maps(pod).await;

        // Check if all required config maps have been created in the api-server
        // Transition pod to retry state if some are missing or we geta kube error when
        // communicating with the api server
        let config_map_data = match CreatingConfig::retrieve_config_maps(
            client.clone(),
            pod.namespace(),
            referenced_config_maps,
        )
        .await
        {
            Ok(config_maps) => config_maps,
            Err(MissingConfigMapsError {
                missing_config_maps,
            }) => {
                warn!(
                    "Unable to find all required config maps for service {}, missing: {:?}",
                    pod_state.service_name, &missing_config_maps
                );
                return Transition::next(
                    self,
                    WaitingConfigMap {
                        missing_config_maps,
                    },
                );
            }
            Err(e) => {
                // Not sure, shouldn't really happen, just do what we know: wait
                return Transition::next(
                    self,
                    WaitingConfigMap {
                        missing_config_maps: vec![format!(
                            "An unexepected error occurred: {:?}",
                            e
                        )],
                    },
                );
            }
        };

        // At this point we have all config maps and their content that we need, otherwise the
        // error cases in the above match statement would have moved the pod to the waiting for
        // configmap state already

        let template_data = if let Ok(data) = CreatingConfig::create_render_data(&pod_state) {
            data
        } else {
            error!("Unable to parse directories for command template as UTF8");
            return Transition::next(
                self,
                SetupFailed {
                    message: "Unable to parse directories for command template as UTF8".to_string(),
                },
            );
        };

        // Write the config files
        let config_dir = pod_state.get_service_config_directory();
        for (target_path, volume) in volume_mounts {
            let volume = volume.clone();
            let joined_target_path = config_dir.join(&target_path);

            debug!("Applying config map {} to {}", volume, target_path);
            if let Some(volume_content) = config_map_data.get(&volume) {
                if let Err(e) = CreatingConfig::apply_config_map(
                    volume_content,
                    &joined_target_path,
                    &template_data,
                ) {
                    // Creation of config file failed!
                    let error_message = format!(
                        "Failed to create config file [{:?}] from config map [{}] due to: {:?}",
                        &joined_target_path.to_str(),
                        volume,
                        e
                    );
                    error!("{}", &error_message);
                    return Transition::next(
                        self,
                        SetupFailed {
                            message: error_message,
                        },
                    );
                }
            }
            // Creation went well, carry on
        }
        debug!("Transitioning to service creation");
        Transition::next(self, CreatingService)
    }

    async fn json_status(
        &self,
        _pod_state: &mut PodState,
        _pod: &Pod,
    ) -> anyhow::Result<serde_json::Value> {
        make_status(Phase::Pending, "CreatingConfig")
    }
}

#[cfg(test)]
mod tests {
    use crate::provider::states::creating_config::CreatingConfig;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[test]
    fn test_render_template() {
        let mut context = BTreeMap::new();
        context.insert(String::from("var1"), String::from("test"));
        context.insert(String::from("var2"), String::from("test2"));
        context.insert(String::from("var3"), String::from("test3"));

        let template = "{{var1}}test{{var2}}test2{{var3}}test3";
        let rendered_string = "testtesttest2test2test3test3";

        let test = CreatingConfig::render_config_template(&context, template).unwrap();

        // Test if string is rendered correctly
        assert_eq!(test, rendered_string);

        // Test if an undefined variable leads to an error
        let template_with_undefined_var = "{{var4}}test";
        match CreatingConfig::render_config_template(&context, template_with_undefined_var) {
            Ok(_) => assert!(false),
            Err(_) => assert!(true),
        }
    }

    #[test]
    fn test_pathbuf_string_conversion() {
        let input_path_string = "/home/test/.kube/config";
        let legal_path = PathBuf::from(input_path_string);
        let legal_path_string = CreatingConfig::pathbuf_to_string("testfield", legal_path).unwrap();
        assert_eq!(input_path_string, legal_path_string);
    }
}
