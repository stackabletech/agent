use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use kubelet::container::Container;
use kubelet::pod::Pod;

use crate::provider::error::StackableError;
use crate::provider::error::StackableError::PodValidationError;
use crate::provider::kubernetes::accessor::{restart_policy, RestartPolicy};
use crate::provider::states::pod::creating_config::CreatingConfig;
use crate::provider::states::pod::PodState;
use crate::provider::systemdmanager::manager::UnitTypes;
use lazy_static::lazy_static;
use log::{debug, error, info, trace, warn};
use multimap::MultiMap;
use regex::Regex;
use std::fmt;
use std::fmt::{Display, Formatter};
use std::iter::{self, repeat};
use strum::{Display, EnumIter, IntoEnumIterator};

/// The default timeout for stopping a service, after this has passed systemd will terminate
/// the process
const DEFAULT_TERMINATION_TIMEOUT_SECS: i64 = 30;

/// List of sections in the systemd unit
///
/// The sections are written in the same order as listed here into the unit file.
#[derive(Clone, Copy, Debug, Display, EnumIter, Eq, Hash, PartialEq)]
pub enum Section {
    Unit,
    Service,
    Install,
}

lazy_static! {
    // Pattern for user names to comply with the strict mode of systemd
    // see https://systemd.io/USER_NAMES/
    static ref USER_NAME_PATTERN: Regex =
        Regex::new("^[a-zA-Z_][a-zA-Z0-9_-]{0,30}$").unwrap();
}

/// Configures whether the service shall be restarted when the service
/// process exits, is killed, or a timeout is reached.
///
/// The service process may be the main service process, but it may also
/// be one of the processes specified with `ExecStartPre=`,
/// `ExecStartPost=`, `ExecStop=`, `ExecStopPost=`, or `ExecReload=`.
/// When the death of the process is a result of systemd operation (e.g.
/// service stop or restart), the service will not be restarted.
/// Timeouts include missing the watchdog "keep-alive ping" deadline and
/// a service start, reload, and stop operation timeouts.
///
/// As exceptions to the setting, the service will not be restarted if
/// the exit code or signal is specified in `RestartPreventExitStatus=`
/// or the service is stopped with `systemctl stop` or an equivalent
/// operation. Also, the services will always be restarted if the exit
/// code or signal is specified in `RestartForceExitStatus=`.
///
/// Note that service restart is subject to unit start rate limiting
/// configured with `StartLimitIntervalSec=` and `StartLimitBurst=`. A
/// restarted service enters the failed state only after the start
/// limits are reached.
///
/// Setting this to "RestartOption::OnFailure" is the recommended choice
/// for long-running services, in order to increase reliability by
/// attempting automatic recovery from errors. For services that shall
/// be able to terminate on their own choice (and avoid immediate
/// restarting), "RestartOption::OnAbnormal" is an alternative choice.
#[derive(Clone, Debug, Display, Eq, PartialEq)]
#[strum(serialize_all = "kebab-case")]
pub enum RestartOption {
    /// The service will be restarted regardless of whether it exited
    /// cleanly or not, got terminated abnormally by a signal, or hit a
    /// timeout.
    Always,
    /// The service will not be restarted.
    No,
    /// The service will be restarted when the process is terminated by
    /// a signal (including on core dump, excluding the signals
    /// `SIGHUP`, `SIGINT`, `SIGTERM`, or `SIGPIPE`), when an operation
    /// times out, or when the watchdog timeout is triggered.
    OnAbnormal,
    /// The service will be restarted only if the service process exits
    /// due to an uncaught signal not specified as a clean exit status.
    OnAbort,
    /// The service will be restarted when the process exits with a
    /// non-zero exit code, is terminated by a signal (including on core
    /// dump, but excluding the signals `SIGHUP`, `SIGINT`, `SIGTERM`,
    /// or `SIGPIPE`), when an operation (such as service reload) times
    /// out, and when the configured watchdog timeout is triggered.
    OnFailure,
    /// The service will be restarted only when the service process
    /// exits cleanly. In this context, a clean exit means any of the
    /// following:
    /// - exit code of 0;
    /// - for types other than Type=oneshot, one of the signals
    ///   `SIGHUP`, `SIGINT`, `SIGTERM`, or `SIGPIPE`;
    /// - exit statuses and signals specified in SuccessExitStatus=.
    OnSuccess,
    /// The service will be restarted only if the watchdog timeout for
    /// the service expires.
    OnWatchdog,
}

impl From<RestartPolicy> for RestartOption {
    fn from(restart_policy: RestartPolicy) -> Self {
        match restart_policy {
            RestartPolicy::Always => RestartOption::Always,
            RestartPolicy::OnFailure => RestartOption::OnFailure,
            RestartPolicy::Never => RestartOption::OnAbnormal,
        }
    }
}

/// A struct that represents an individual systemd unit
#[derive(Clone, Debug)]
pub struct SystemDUnit {
    pub name: String,
    pub unit_type: UnitTypes,
    pub sections: HashMap<Section, MultiMap<String, String>>,
}

// TODO: The parsing code is also highly stackable specific, we should
//  at some point consider splitting this out and have systemdunit live
//  inside the systemd crate and the parsing in the agent
impl SystemDUnit {
    /// Create a new unit which inherits all common elements from ['common_properties'] and parses
    /// everything else from the ['container']
    pub fn new(
        common_properties: &SystemDUnit,
        name_prefix: &str,
        container: &Container,
        user_mode: bool,
        pod_state: &PodState,
    ) -> Result<Self, StackableError> {
        // Create template data to be used when rendering template strings
        let template_data = if let Ok(data) = CreatingConfig::create_render_data(pod_state) {
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

        let package_root = pod_state.get_service_package_directory();

        SystemDUnit::new_from_container(
            common_properties,
            name_prefix,
            container,
            &pod_state.service_name,
            &template_data,
            &package_root,
            user_mode,
        )
    }

    fn new_from_container(
        common_properties: &SystemDUnit,
        name_prefix: &str,
        container: &Container,
        service_name: &str,
        template_data: &BTreeMap<String, String>,
        package_root: &Path,
        user_mode: bool,
    ) -> Result<Self, StackableError> {
        let mut unit = common_properties.clone();

        let trimmed_name = match container
            .name()
            .strip_suffix(common_properties.get_type_string())
        {
            None => container.name().to_string(),
            Some(name_without_suffix) => name_without_suffix.to_string(),
        };

        unit.name = format!("{}{}", name_prefix, trimmed_name);

        unit.set_property(Section::Unit, "Description", &unit.name.clone());

        unit.set_property(
            Section::Service,
            "ExecStart",
            &SystemDUnit::get_command(container, template_data, package_root)?,
        );

        let env_vars = SystemDUnit::get_environment(container, service_name, template_data)?;
        for (key, value) in env_vars {
            unit.add_env_var(&key, &value);
        }

        // These are currently hard-coded, as this is not something we expect to change soon
        unit.set_property(Section::Service, "StandardOutput", "journal");
        unit.set_property(Section::Service, "StandardError", "journal");

        if let Some(user_name) =
            SystemDUnit::get_user_name_from_security_context(container, &unit.name)?
        {
            if !user_mode {
                unit.set_property(Section::Service, "User", user_name);
            } else {
                info!("The user name [{}] in spec.containers[name = {}].securityContext.windowsOptions.runAsUserName is not set in the systemd unit because the agent runs in session mode.", user_name, container.name());
            }
        }

        // This one is mandatory, as otherwise enabling the unit fails
        unit.set_property(Section::Install, "WantedBy", "multi-user.target");

        Ok(unit)
    }

    fn get_user_name_from_security_context<'a>(
        container: &'a Container,
        pod_name: &str,
    ) -> Result<Option<&'a str>, StackableError> {
        let validate = |user_name| {
            if USER_NAME_PATTERN.is_match(user_name) {
                Ok(user_name)
            } else {
                Err(PodValidationError {
                    msg: format!(
                        r#"The validation of the pod [{}] failed. The user name [{}] in spec.containers[name = {}].securityContext.windowsOptions.runAsUserName must match the regular expression "{}"."#,
                        pod_name,
                        user_name,
                        container.name(),
                        USER_NAME_PATTERN.to_string()
                    ),
                })
            }
        };

        container
            .security_context()
            .and_then(|security_context| security_context.windows_options.as_ref())
            .and_then(|windows_options| windows_options.run_as_user_name.as_ref())
            .map(|user_name| validate(user_name))
            .transpose()
    }

    /// Parse a pod object and retrieve the generic settings which will be the same across
    /// all service units created for containers in this pod.
    /// This is designed to then be used as `common_properties` parameter when calling
    ///[`SystemDUnit::new`]
    pub fn new_from_pod(pod: &Pod, user_mode: bool) -> Result<Self, StackableError> {
        let mut unit = SystemDUnit {
            name: pod.name().to_string(),
            unit_type: UnitTypes::Service,
            sections: Default::default(),
        };

        // Kubernetes does not allow creating pods without a spec, so if we do not get one here
        //something is definitely seriously amiss
        let pod_spec = match &pod.as_kube_pod().spec {
            Some(spec) => spec,
            None => {
                return Err(PodValidationError {
                    msg: format!("Got pod without spec: [{}]", unit.name),
                })
            }
        };

        // If `terminationGracePeriodSeconds` was specified in the PodSpec set the value as
        // 'TimeOutStopSec` on the systemd unit
        // This means that the service will be killed after this period if it does not shutdown
        // after receiving a stop command
        // If it was not specified we use the default value for 'terminationGracePeriodSeconds' of
        // 30 seconds, as this differs from the systemd default for 'TimeOutStopSec` which is 90
        // seconds.
        let termination_timeout = match pod_spec.termination_grace_period_seconds {
            None => DEFAULT_TERMINATION_TIMEOUT_SECS,
            Some(specified_timeout) => specified_timeout,
        }
        .to_string();

        unit.set_property(Section::Service, "TimeoutStopSec", &termination_timeout);

        unit.set_restart_option(RestartOption::from(restart_policy(&pod)));

        if let Some(user_name) = SystemDUnit::get_user_name_from_pod_security_context(pod)? {
            if !user_mode {
                unit.set_property(Section::Service, "User", user_name);
            } else {
                info!("The user name [{}] in spec.securityContext.windowsOptions.runAsUserName is not set in the systemd unit because the agent runs in session mode.", user_name);
            }
        }

        Ok(unit)
    }

    fn set_restart_option(&mut self, setting: RestartOption) {
        self.set_property(Section::Service, "Restart", &setting.to_string());
    }

    fn get_user_name_from_pod_security_context(pod: &Pod) -> Result<Option<&str>, StackableError> {
        let validate = |user_name| {
            if USER_NAME_PATTERN.is_match(user_name) {
                Ok(user_name)
            } else {
                Err(PodValidationError {
                    msg: format!(
                        r#"The validation of the pod [{}] failed. The user name [{}] in spec.securityContext.windowsOptions.runAsUserName must match the regular expression "{}"."#,
                        pod.name(),
                        user_name,
                        USER_NAME_PATTERN.to_string()
                    ),
                })
            }
        };

        pod.as_kube_pod()
            .spec
            .as_ref()
            .and_then(|spec| spec.security_context.as_ref())
            .and_then(|security_context| security_context.windows_options.as_ref())
            .and_then(|windows_options| windows_options.run_as_user_name.as_ref())
            .map(|user_name| validate(user_name))
            .transpose()
    }

    /// Convenience function to retrieve the _fully qualified_ systemd name, which includes the
    /// `.servicetype` part.
    pub fn get_name(&self) -> String {
        let lower_type = format!("{:?}", self.unit_type).to_lowercase();
        format!("{}.{}", self.name, lower_type)
    }

    /// Adds an environment variable to the service section of the unit file
    pub fn add_env_var(&mut self, key: &str, value: &str) {
        self.add_property(
            Section::Service,
            "Environment",
            &format!("\"{}={}\"", key, value),
        );
    }

    /// Sets a property in the given section
    ///
    /// If properties with the given key already exist then they are
    /// replaced with the given one.
    fn set_property(&mut self, section: Section, key: &str, value: &str) {
        let section = self.sections.entry(section).or_default();
        *section.entry(String::from(key)).or_insert_vec(Vec::new()) = vec![String::from(value)];
    }

    /// Adds a property to the given section
    ///
    /// Properties with the same key remain untouched.
    fn add_property(&mut self, section: Section, key: &str, value: &str) {
        let section = self.sections.entry(section).or_default();
        section.insert(String::from(key), String::from(value));
    }

    /// Retrieve content of the unit file as it should be written to disk
    pub fn get_unit_file_content(&self) -> String {
        Section::iter()
            .map(|section| self.sections.get_key_value(&section))
            .flatten()
            .map(|(section, entries)| SystemDUnit::write_section(section, entries))
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    fn write_section(section: &Section, entries: &MultiMap<String, String>) -> String {
        let header = format!("[{}]", section);

        let mut body = entries
            .iter_all()
            .flat_map(|(key, values)| repeat(key).zip(values))
            .map(|(key, value)| format!("{}={}", key, value))
            .collect::<Vec<_>>();
        body.sort();

        iter::once(header)
            .chain(body)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn get_type_string(&self) -> &str {
        match &self.unit_type {
            UnitTypes::Service => ".service",
        }
    }

    fn get_environment(
        container: &Container,
        service_name: &str,
        template_data: &BTreeMap<String, String>,
    ) -> Result<Vec<(String, String)>, StackableError> {
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
            debug!("Got environment vars: {:?} service {}", vars, service_name);
            let render_result = vars
                .iter()
                .map(|env_var| {
                    // Replace variables in value
                    CreatingConfig::render_config_template(
                        template_data,
                        env_var.value.as_deref().unwrap_or_default(),
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
            debug!("No environment vars set for service {}", service_name);
            vec![]
        };
        debug!(
            "Setting environment for service {} to {:?}",
            service_name, &env_variables
        );

        Ok(env_variables)
    }

    // Retrieve a copy of the command object in the pod, or return an error if it is missing
    fn get_command(
        container: &Container,
        template_data: &BTreeMap<String, String>,
        package_root: &Path,
    ) -> Result<String, StackableError> {
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
            .map(|command_part| CreatingConfig::render_config_template(template_data, command_part))
            .collect::<Result<Vec<String>, StackableError>>()?;

        trace!(
            "Command after replacing variables and adding packageroot: {:?}",
            command_render_result
        );

        Ok(command_render_result.join(" "))
    }
}

impl Display for SystemDUnit {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.get_name())
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::provider::test::TestPod;
    use indoc::indoc;
    use rstest::rstest;
    use std::path::PathBuf;

    #[derive(PartialEq)]
    enum BusType {
        Session,
        System,
    }

    #[rstest]
    #[case::without_containers_on_system_bus(
        BusType::System,
        "
            apiVersion: v1
            kind: Pod
            metadata:
              name: stackable
            spec:
              containers: []
              securityContext:
                windowsOptions:
                  runAsUserName: pod-user",
        "stackable.service",
        indoc! {"
            [Service]
            Restart=always
            TimeoutStopSec=30
            User=pod-user"}
    )]
    #[case::with_container_on_system_bus(
        BusType::System,
        r#"
            apiVersion: v1
            kind: Pod
            metadata:
              name: stackable
            spec:
              containers:
                - name: test-container.service
                  command:
                    - start.sh
                  args:
                    - arg
                    - "{{configroot}}"
                  env:
                    - name: LOG_LEVEL
                      value: INFO
                    - name: LOG_DIR
                      value: "{{logroot}}"
                  securityContext:
                    windowsOptions:
                      runAsUserName: container-user
              securityContext:
                windowsOptions:
                  runAsUserName: pod-user"#,
        "default-stackable-test-container.service",
        indoc! {r#"
            [Unit]
            Description=default-stackable-test-container

            [Service]
            Environment="LOG_DIR=/var/log/default-stackable"
            Environment="LOG_LEVEL=INFO"
            ExecStart=start.sh arg /etc/default-stackable
            Restart=always
            StandardError=journal
            StandardOutput=journal
            TimeoutStopSec=30
            User=container-user

            [Install]
            WantedBy=multi-user.target"#}
    )]
    #[case::with_container_on_session_bus(
        BusType::Session,
        r#"
            apiVersion: v1
            kind: Pod
            metadata:
              name: stackable
            spec:
              containers:
                - name: test-container.service
                  command:
                    - start.sh
                  securityContext:
                    windowsOptions:
                      runAsUserName: container-user
              securityContext:
                windowsOptions:
                  runAsUserName: pod-user"#,
        "default-stackable-test-container.service",
        indoc! {r#"
            [Unit]
            Description=default-stackable-test-container

            [Service]
            ExecStart=start.sh
            Restart=always
            StandardError=journal
            StandardOutput=journal
            TimeoutStopSec=30

            [Install]
            WantedBy=multi-user.target"#}
    )]
    #[case::set_termination_timeout(
        BusType::System,
        "
            apiVersion: v1
            kind: Pod
            metadata:
              name: stackable
            spec:
              terminationGracePeriodSeconds: 10
              containers: []",
        "stackable.service",
        indoc! {"
            [Service]
            Restart=always
            TimeoutStopSec=10"}
    )]
    #[case::set_restart_policy(
        BusType::System,
        "
            apiVersion: v1
            kind: Pod
            metadata:
              name: stackable
            spec:
              containers: []
              restartPolicy: OnFailure",
        "stackable.service",
        indoc! {"
            [Service]
            Restart=on-failure
            TimeoutStopSec=30"
        }
    )]

    fn create_unit_from_pod(
        #[case] bus_type: BusType,
        #[case] pod: TestPod,
        #[case] expected_unit_file_name: &str,
        #[case] expected_unit_file_content: &str,
    ) {
        let mut result = SystemDUnit::new_from_pod(&pod, bus_type == BusType::Session);

        if let Ok(common_properties) = &result {
            if let Some(container) = pod.containers().first() {
                let service_name = format!("{}-{}", pod.namespace(), pod.name());
                let name_prefix = format!("{}-", service_name);
                let mut template_data = BTreeMap::new();
                template_data.insert(
                    String::from("logroot"),
                    format!("/var/log/{}", &service_name),
                );
                template_data.insert(
                    String::from("configroot"),
                    format!("/etc/{}", &service_name),
                );
                let package_root = PathBuf::new();

                result = SystemDUnit::new_from_container(
                    common_properties,
                    &name_prefix,
                    container,
                    &service_name,
                    &template_data,
                    &package_root,
                    bus_type == BusType::Session,
                );
            }
        }

        if let Ok(unit) = result {
            assert_eq!(expected_unit_file_name, unit.get_name());
            assert_eq!(expected_unit_file_content, unit.get_unit_file_content());
        } else {
            panic!("Systemd unit expected but got {:?}", result);
        }
    }
}
