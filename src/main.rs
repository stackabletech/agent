use std::path::PathBuf;

use kube::config::Config as KubeConfig;
use kube::config::KubeConfigOptions;
use kubelet::config::Config;
use kubelet::Kubelet;

use crate::provider::StackableProvider;
use pnet::datalink;
use std::net::IpAddr;

mod provider;

#[tokio::main(threaded_scheduler)]
async fn main() -> anyhow::Result<()> {
    // The provider is responsible for all the "back end" logic. If you are creating
    // a new Kubelet, all you need to implement is a provider.

    // Lookup the default IP Adress and export this to the environment for the Krustlet config
    // to pick up
    if let Some(default_ipv4_address) = get_default_ipaddress() {
        std::env::set_var("KRUSTLET_NODE_IP", default_ipv4_address.to_string());
    }

    let config = Config::new_from_flags(env!("CARGO_PKG_VERSION"));

    // Initialize the logger
    env_logger::init();

    //let kubeconfig = kubelet::bootstrap(&config, &config.bootstrap_file, notify_bootstrap).await?;
    let kubeconfig = KubeConfig::from_kubeconfig(&KubeConfigOptions::default())
        .await
        .expect("Failed to create Kubernetes Client!");

    let parcel_directory = PathBuf::from("/home/sliebau/IdeaProjects/agent/work/parcels");
    let config_directory = PathBuf::from("/home/sliebau/IdeaProjects/agent/work/config");
    let provider = StackableProvider::new(
        kube::Client::new(kubeconfig.clone()),
        parcel_directory,
        config_directory,
    )
    .await
    .expect("Error initializing provider.");

    let kubelet = Kubelet::new(provider, kubeconfig, config).await?;
    kubelet.start().await
}

fn get_default_ipaddress() -> Option<IpAddr> {
    let all_interfaces = datalink::interfaces();

    let default_interface = all_interfaces
        .iter()
        .filter(|e| e.is_up() && !e.is_loopback() && e.ips.len() > 0)
        .next();

    match default_interface {
        Some(interface) => {
            println!("Found default interface with [{:?}].", interface.ips);
            if let ipv4_network = interface.ips[0] {
                return Some(ipv4_network.ip());
            }
        }
        None => println!("Error while finding the default interface."),
    };
    return None;
}
