use std::collections::HashMap;

use kubelet::container::Container;
use kubelet::pod::Pod;
use phf;
use phf::{Map, OrderedSet};

use crate::provider::error::StackableError;

use crate::provider::error::StackableError::PodValidationError;
use crate::provider::states::creating_config::CreatingConfig;
use crate::provider::systemdmanager::manager::UnitTypes;
use crate::provider::PodState;
use log::{debug, error, trace, warn};

static RESTART_POLICY_MAP: Map<&'static str, &'static str> = phf::phf_map! {
    "Always" => "always",
    "OnFailure" => "on-failure",
    "Never" => "no",
};

pub const SECTION_SERVICE: &str = "Service";
pub const SECTION_UNIT: &str = "Unit";
pub const SECTION_INSTALL: &str = "Install";

// TODO: This will be used later to ensure the same ordering of known sections in
// unit files, I'll leave it in for now
#[allow(dead_code)]
static SECTION_ORDER: OrderedSet<&'static str> =
    phf::phf_ordered_set! {"Unit", "Service", "Install"};

/// A struct representing all systemd units that this server needs to run for a pod that
/// it was assigned.
///
/// There will be a 1:1 relationship between pods assigned to this node and objects of this
/// struct.
///
/// Within the struct multiple systemd units can exist, depending on the container configuration
/// in the pod.
///
/// Currently only service units are implemented, but we plan to extend this to other types of
/// of units as well (mounts, ..)
///
/// Init Containers will be translated to service units as well (still TODO)
pub struct Service {
    pub systemd_units: Vec<SystemDUnit>,
}

impl Service {
    pub fn new(pod: &Pod, pod_state: &PodState) -> Result<Self, StackableError> {
        // Create systemd unit template with values we need from the pod object
        let pod_settings = SystemDUnit::new_from_pod(&pod)?;

        // Convert all containers to systemd unit files
        let systemd_units: Result<Vec<SystemDUnit>, StackableError> = pod
            .containers()
            .iter()
            .map(|container| SystemDUnit::new(&pod_settings, container, pod_state))
            .collect();

        systemd_units.map(|units| Self {
            systemd_units: units,
        })
    }
}

/// A struct that represents an individual systemd unit
#[derive(Clone)]
pub struct SystemDUnit {
    name: String,
    unit_type: UnitTypes,
    sections: HashMap<String, HashMap<String, String>>,
    environment: HashMap<String, String>,
}

impl SystemDUnit {
    /// Create a new unit which inherits all common elements from ['common_properties'] and parses
    /// everything else from the ['container']
    pub fn new(
        common_properties: &SystemDUnit,
        container: &Container,
        pod_state: &PodState,
    ) -> Result<Self, StackableError> {
        let mut unit = common_properties.clone();
        unit.name = String::from(container.name());

        unit.add_property(SECTION_UNIT, "Description", &unit.name.clone());

        unit.add_property(
            SECTION_SERVICE,
            "ExecStart",
            &SystemDUnit::get_command(container, pod_state)?,
        );

        let env_vars = SystemDUnit::get_environment(container, pod_state)?;

        for (name, value) in env_vars {
            unit.add_env_var(&name, &value);
        }

        unit.add_property(SECTION_SERVICE, "StandardOutput", "journal");
        unit.add_property(SECTION_SERVICE, "StandardError", "journal");
        unit.add_property(SECTION_INSTALL, "WantedBy", "multi-user.target");

        Ok(unit)
    }

    pub fn new_from_pod(pod: &Pod) -> Result<Self, StackableError> {
        let mut unit = SystemDUnit {
            name: pod.name().to_string(),
            unit_type: UnitTypes::Service,
            sections: Default::default(),
            environment: Default::default(),
        };

        let restart_policy = match &pod.as_kube_pod().spec {
            // if no restart policy is present we default to "never"
            Some(spec) => spec.restart_policy.as_deref().unwrap_or("Never"),
            None => "Never",
        };

        // if however one is specified but we do not know about this policy then we do not default
        // to never but fail the service instead to avoid unpredictable behavior
        let restart_policy = match RESTART_POLICY_MAP.get(restart_policy) {
            Some(policy) => policy,
            None => {
                return Err(PodValidationError {
                    msg: format!(
                        "Unknown value [{}] for RestartPolicy in pod [{}]",
                        restart_policy, unit.name
                    ),
                })
            }
        };

        unit.add_property(SECTION_SERVICE, "Restart", restart_policy);
        Ok(unit)
    }

    /// Add a key=value entry to the specified section
    fn add_property(&mut self, section: &'static str, key: &str, value: &str) {
        let section = self
            .sections
            .entry(String::from(section))
            .or_insert_with(HashMap::new);
        section.insert(String::from(key), String::from(value));
    }

    fn add_env_var(&mut self, name: &str, value: &str) {
        self.environment
            .insert(String::from(name), String::from(value));
    }

    pub fn get_unit_file_content(&self) -> String {
        let mut unit_file_content = String::new();

        // Iterate over all sections and write out its header and content
        for (section, entries) in &self.sections {
            unit_file_content.push_str(&format!("[{}]\n", section));
            for (key, value) in entries {
                unit_file_content.push_str(&format!("{}={}\n", key, value));
            }
            if section == SECTION_SERVICE {
                // Add environment variables to Service section
                for (name, value) in &self.environment {
                    unit_file_content.push_str(&format!("Environment=\"{}={}\"\n", name, value));
                }
            }
            unit_file_content.push('\n');
        }
        unit_file_content
    }

    fn get_environment(
        container: &Container,
        pod_state: &PodState,
    ) -> Result<Vec<(String, String)>, StackableError> {
        // Create template data to be used when rendering template strings
        let template_data = if let Ok(data) = CreatingConfig::create_render_data(&pod_state) {
            data
        } else {
            error!("Unable to parse directories for command template as UTF8");
            return Err(PodValidationError {
                msg: format!(
                    "Unable to parse directories for command template as UTF8 for container [{}].",
                    container.name()
                ),
            });
        };

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
                    .map(|value| (env_var.name.clone(), value))
                })
                .collect();

            // If any single rendering failed, the overall result for the map will have
            // collected the Err which we can check for here
            match render_result {
                Ok(rendered_values) => rendered_values,
                Err(error) => {
                    error!("Failed to render value for env var due to: {:?}", error);
                    return Err(PodValidationError {
                        msg: String::from("Failed to render a template"),
                    });
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

        Ok(env_variables)
    }

    // Retrieve a copy of the command object in the pod, or return an error if it is missing
    fn get_command(container: &Container, pod_state: &PodState) -> Result<String, StackableError> {
        // Return an error if no command was specified in the container
        // TODO: We should discuss if there can be a valid scenario for this
        // This clones because we perform some in place mutations on the elements
        let mut command = match container.command() {
            Some(command) => command.clone(),
            _ => {
                return Err(PodValidationError {
                    msg: format!(
                    "Error creating systemd unit for container {}, due to missing command element.",
                    container.name()
                ),
                })
            }
        };

        let package_root = pod_state.get_service_package_directory();

        trace!(
            "Command before replacing variables and adding packageroot: {:?}",
            command
        );
        // Get a mutable reference to the first element of the command array as we might need to
        // add the package directory to this to make it an absolute path
        let binary = match command.get_mut(0) {
            Some(binary_string) => binary_string,
            None => {
                return Err(PodValidationError {
                    msg: format!(
                        "Unable to convert command for container [{}] to utf8.",
                        container.name()
                    ),
                })
            }
        };

        // Warn if the user tried to add the packageroot directory to the command themselves
        // This warning only triggers if the command starts with the packageroot as this is the
        // only hard coded replacement we perform
        // It might be perfectly reasonable to reference the packageroot directory somewhere
        // later on in the command
        if binary.starts_with("{{packageroot}}") {
            warn!("Command for [{}] starts with \"{{packageroot}}\" - this would usually be automatically prepended to the command. Skipping prepending the directory and relying on string replacement instead, which is not recommended!", container.name());
        } else {
            // Prepend package root to first element of the command array, which should be the binary
            // this service has to execute
            debug!(
                "Prepending [{:?}] as package directory to the command for container [{}]",
                package_root,
                container.name()
            );
            let binary_with_path = match package_root.join(&binary).into_os_string().into_string() {
                Ok(path_string) => path_string,
                Err(_) => {
                    return Err(PodValidationError {
                        msg: format!(
                            "Unable to convert command for container [{}] to utf8.",
                            container.name()
                        ),
                    })
                }
            };
            binary.replace_range(.., &binary_with_path);
        }

        // Create template data to be used when rendering template strings
        let template_data = if let Ok(data) = CreatingConfig::create_render_data(&pod_state) {
            data
        } else {
            error!("Unable to parse directories for command template as UTF8");
            return Err(PodValidationError {
                msg: format!(
                    "Unable to parse directories for command template as UTF8 for container [{}].",
                    container.name()
                ),
            });
        };

        // Append values from args array to command array
        // This is necessary as we only have the ExecStart field in a systemd service unit.
        // There is no specific place to put arguments separate from the command.
        if let Some(mut args) = container.args().clone() {
            debug!(
                "Appending arguments [{:?}] to command for [{}]",
                args,
                container.name()
            );
            command.append(args.as_mut());
        }

        // Replace variables in command array
        let command_render_result = command
            .iter()
            .map(|command_part| {
                CreatingConfig::render_config_template(&template_data, command_part)
            })
            .collect::<Result<Vec<String>, StackableError>>()?;

        trace!(
            "Command after replacing variables and adding packageroot: {:?}",
            command_render_result
        );

        Ok(command_render_result.join(" "))
    }
}
