use std::env;
use std::ffi::OsString;
use std::net::IpAddr;
use std::path::PathBuf;

use kube::config::Config as KubeConfig;
use kube::config::KubeConfigOptions;
use kubelet::config::{Config, ServerConfig};
use kubelet::Kubelet;
use log::{debug, info, warn};
use pnet::datalink;
use stackable_config::ConfigBuilder;

use crate::agentconfig::AgentConfig;
use crate::provider::StackableProvider;

mod agentconfig;
mod provider;

#[tokio::main(threaded_scheduler)]
async fn main() -> anyhow::Result<()> {
    // Initialize the logger
    env_logger::init();

    // The provider is responsible for all the "back end" logic. If you are creating
    // a new Kubelet, all you need to implement is a provider.

    let agent_config: AgentConfig =
        *ConfigBuilder::build(env::args_os().collect::<Vec<OsString>>(), "CONFIG_FILE")
            .expect("Error initializing Configuration!");

    export_env(
        "KRUSTLET_NODE_IP",
        &agent_config.server_ip_address.to_string(),
    );

    // Convert node tags to string in the form of key=value,key=value,...
    let node_labels = agent_config
        .tags
        .iter()
        .map(|(k, v)| format!("{}={}", String::from(k), String::from(v)))
        .collect::<Vec<_>>()
        .join(",");

    export_env("NODE_LABELS", &node_labels);

    if let Some(cert_file_path) = agent_config.server_cert_file {
        export_env("KRUSTLET_CERT_FILE", cert_file_path.to_str().unwrap());
    } else {
        warn!("Not exporting server cert file path, as non was specified that could be converted to a String.");
    }

    if let Some(key_file_path) = agent_config.server_key_file {
        export_env("KRUSTLET_PRIVATE_KEY_FILE", key_file_path.to_str().unwrap());
    } else {
        warn!("Not exporting server key file path, as non was specified that could be converted to a String.");
    }
    info!("args: {:?}", env::args());
    let krustlet_config = Config::new_from_flags(env!("CARGO_PKG_VERSION"));

    //let kubeconfig = kubelet::bootstrap(&config, &config.bootstrap_file, notify_bootstrap).await?;
    let kubeconfig = KubeConfig::from_kubeconfig(&KubeConfigOptions::default())
        .await
        .expect("Failed to create Kubernetes Client!");

    //let parcel_directory = PathBuf::from("/home/sliebau/IdeaProjects/agent/work/parcels");
    //let config_directory = PathBuf::from("/home/sliebau/IdeaProjects/agent/work/config");
    let provider = StackableProvider::new(
        kube::Client::new(kubeconfig.clone()),
        agent_config.parcel_directory.clone(),
        agent_config.config_directory.clone(),
        agent_config.log_directory.clone(),
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
