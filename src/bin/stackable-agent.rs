use std::collections::HashMap;
use std::env;
use std::ffi::OsString;
use std::io::ErrorKind;
use std::path::PathBuf;

use kubelet::config::{Config, ServerConfig};
use kubelet::Kubelet;
use log::{error, info};
use tokio::fs::File;

use stackable_agent::config::AgentConfig;
use stackable_agent::fsext::check_dir_is_writable;
use stackable_agent::provider::StackableProvider;
use stackable_config::{ConfigBuilder, ConfigOption};

mod built_info {
    // The file has been placed there by the build script.
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}

pub fn print_startup_string(
    pkg_version: &str,
    git_version: Option<&str>,
    target: &str,
    built_time: &str,
    rustc_version: &str,
) {
    let git_information = match git_version {
        None => "".to_string(),
        Some(git) => format!(" (Git information: {})", git),
    };
    info!("Starting the Stackable Agent");
    info!(
        "This is version {}{}, built for {} by {} at {}",
        pkg_version, git_information, target, rustc_version, built_time
    )
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize the logger
    env_logger::init();

    let agent_config: AgentConfig =
        ConfigBuilder::build(env::args_os().collect::<Vec<OsString>>(), "CONFIG_FILE")
            .expect("Error initializing Configuration!");

    // Make sure to only print diagnostic information once we are actually trying to start
    print_startup_string(
        built_info::PKG_VERSION,
        built_info::GIT_VERSION,
        built_info::TARGET,
        built_info::BUILT_TIME_UTC,
        built_info::RUSTC_VERSION,
    );

    check_optional_files(&agent_config).await;
    check_configured_directories(&agent_config).await;

    // Currently the only way to _properly_ configure the Krustlet is via these environment exports,
    // as their config object only offers methods that parse from command line flags (or combinations
    // of those flags with other things).
    // Since we have our own command line flags that are not compatible with the Krustlet's we
    // configure the agent via a file from the environment variable (CONFIG_FILE), extract what
    // is needed for the Krustlet and pass it via environment variables.
    // This is an ugly hack for now, until we've had time to take a proper look at Krustlet's config
    export_env(
        "KRUSTLET_NODE_IP",
        &agent_config.server_ip_address.to_string(),
    );

    // Convert node tags to string in the form of key=value,key=value,...
    // TODO: check for commas in the key value pairs themselves https://github.com/stackabletech/agent/issues/195
    let node_labels = agent_config
        .tags
        .iter()
        .map(|(k, v)| format!("{}={}", String::from(k), String::from(v)))
        .collect::<Vec<_>>()
        .join(",");

    export_env("NODE_LABELS", &node_labels);

    export_env(
        "KRUSTLET_CERT_FILE",
        agent_config.server_cert_file.to_str().unwrap(),
    );

    export_env(
        "KRUSTLET_PRIVATE_KEY_FILE",
        agent_config.server_key_file.to_str().unwrap(),
    );

    info!("args: {:?}", env::args());

    let server_config = ServerConfig {
        addr: agent_config.server_ip_address,
        port: agent_config.server_port,
        cert_file: agent_config.server_cert_file.clone(),
        private_key_file: agent_config.server_key_file.clone(),
    };

    let plugins_directory = agent_config.data_directory.join("plugins");

    let krustlet_config = Config {
        node_ip: agent_config.server_ip_address,
        hostname: agent_config.hostname.to_owned(),
        node_name: agent_config.hostname.to_owned(),
        server_config,
        data_dir: agent_config.data_directory.to_owned(),
        plugins_dir: plugins_directory.to_owned(),
        node_labels: agent_config.tags.to_owned(),
        max_pods: 110,
        bootstrap_file: agent_config.bootstrap_file.to_owned(),
        allow_local_modules: false,
        insecure_registries: None,
    };

    // Bootstrap a kubernetes config, if no valid config is found
    // This also generates certificates for the webserver the krustlet
    // runs
    let kubeconfig = kubelet::bootstrap(
        &krustlet_config,
        &krustlet_config.bootstrap_file,
        notify_bootstrap,
    )
    .await?;

    let provider = StackableProvider::new(
        kube::Client::new(kubeconfig.clone()),
        &agent_config,
        krustlet_config.max_pods,
    )
    .await
    .expect("Error initializing provider.");

    let kubelet = Kubelet::new(provider, kubeconfig, krustlet_config).await?;
    kubelet.start().await
}

fn export_env(var_name: &str, var_value: &str) {
    info!("Exporting {}={}", var_name, var_value);
    std::env::set_var(var_name, var_value);
}

fn notify_bootstrap(message: String) {
    info!("Successfully bootstrapped TLS certificate: {}", message);
}

/// Checks if the optional files can be opened if they exist. An error
/// is logged if they cannot be opened.
async fn check_optional_files(config: &AgentConfig) {
    for (config_option, file) in [
        (AgentConfig::SERVER_CERT_FILE, &config.server_cert_file),
        (AgentConfig::SERVER_KEY_FILE, &config.server_key_file),
    ] {
        if file.is_file() {
            if let Err(error) = File::open(file).await {
                error!(
                    "Could not open file [{}] which is specified in \
                    the configuration option [{}]. {}",
                    file.to_string_lossy(),
                    config_option.name,
                    error
                );
            }
        }
    }
}

/// Checks the configured directories if they are writable by the
/// current process. If this is not the case then errors are logged.
///
/// This check is performed for informational purposes only. The process
/// is intentionally not terminated on failure because there can be
/// false positives, e.g. if the underlying file system does not support
/// temporary files which are used for the check.
///
/// A successful check also does not guarantee that the process can
/// write to the directory at a later time, e.g. if permissions are
/// changed or a quota is hit.
async fn check_configured_directories(config: &AgentConfig) {
    for (config_option, directory) in directories_where_write_access_is_required(config).await {
        let directory = if directory.components().count() == 0 {
            PathBuf::from(".")
        } else {
            directory
        };

        if let Err(error) = check_dir_is_writable(&directory).await {
            match error.kind() {
                ErrorKind::NotFound => error!(
                    "The directory [{}] specified in the configuration \
                    option [{}] does not exist.",
                    directory.to_string_lossy(),
                    config_option.name
                ),
                ErrorKind::PermissionDenied => error!(
                    "The directory [{}] specified in the configuration \
                    option [{}] is not writable by the process.",
                    directory.to_string_lossy(),
                    config_option.name
                ),
                _ => error!(
                    "An IO error occurred while checking the directory \
                    [{}] specified in the configuration option [{}]. \
                    {}",
                    directory.to_string_lossy(),
                    config_option.name,
                    error
                ),
            };
        }
    }
}

/// Returns all directories configured in the given `AgentConfig` where
/// write access is required.
///
/// The directories of the certificate and key files are only returned
/// if they do not already exist.
async fn directories_where_write_access_is_required(
    config: &AgentConfig,
) -> HashMap<&ConfigOption, PathBuf> {
    let mut dirs = HashMap::new();
    dirs.insert(
        &AgentConfig::PACKAGE_DIR,
        config.parcel_directory.to_owned(),
    );
    dirs.insert(&AgentConfig::CONFIG_DIR, config.config_directory.to_owned());
    dirs.insert(&AgentConfig::LOG_DIR, config.log_directory.to_owned());
    dirs.insert(&AgentConfig::DATA_DIR, config.data_directory.to_owned());

    if !config.server_cert_file.is_file() {
        dirs.insert(
            &AgentConfig::SERVER_CERT_FILE,
            config.server_cert_file_dir().into(),
        );
    }
    if !config.server_key_file.is_file() {
        dirs.insert(
            &AgentConfig::SERVER_KEY_FILE,
            config.server_key_file_dir().into(),
        );
    }

    dirs
}
