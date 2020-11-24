use std::path::PathBuf;

use kube::config::Config as KubeConfig;
use kube::config::KubeConfigOptions;
use kubelet::config::Config;
use kubelet::Kubelet;

use crate::provider::StackableProvider;

mod provider;

#[tokio::main(threaded_scheduler)]
async fn main() -> anyhow::Result<()> {
    // The provider is responsible for all the "back end" logic. If you are creating
    // a new Kubelet, all you need to implement is a provider.
    let config = Config::new_from_file_and_flags(env!("CARGO_PKG_VERSION"), None);

    // Initialize the logger
    env_logger::init();

    //let kubeconfig = kubelet::bootstrap(&config, &config.bootstrap_file, notify_bootstrap).await?;
    let kubeconfig = KubeConfig::from_kubeconfig(&KubeConfigOptions::default())
        .await
        .expect("Failed to create Kubernetes Client!");

    let parcel_directory = PathBuf::from("/home/sliebau/IdeaProjects/agent/work/parcels");
    let config_directory = PathBuf::from("/home/sliebau/IdeaProjects/agent/work/config");
    let provider = StackableProvider::new(kube::Client::new(kubeconfig.clone()), parcel_directory, config_directory)
        .await
        .expect("Error initializing provider.");

    let kubelet = Kubelet::new(provider, kubeconfig, config).await?;
    kubelet.start().await
}
