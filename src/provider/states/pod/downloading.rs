use std::path::Path;

use anyhow::Context;
use kubelet::pod::state::prelude::*;
use kubelet::pod::Pod;
use log::{debug, error, info, warn};
use tokio::fs::create_dir_all;

use super::downloading_backoff::DownloadingBackoff;
use super::installing::Installing;
use crate::provider::repository::find_repository;
use crate::provider::repository::package::Package;
use crate::provider::{PodState, ProviderState};

#[derive(Default, Debug, TransitionTo)]
#[transition_to(Installing, DownloadingBackoff)]
pub struct Downloading;

impl Downloading {
    fn package_downloaded<T: Into<Package>>(package: T, download_directory: &Path) -> bool {
        let package = package.into();
        let package_file_name = download_directory.join(package.get_file_name());
        debug!(
            "Checking if package {} has already been downloaded to {:?}",
            package, package_file_name
        );
        Path::new(&package_file_name).exists()
    }
}

#[async_trait::async_trait]
impl State<PodState> for Downloading {
    async fn next(
        self: Box<Self>,
        provider_state: SharedState<ProviderState>,
        pod_state: &mut PodState,
        _pod: Manifest<Pod>,
    ) -> Transition<PodState> {
        let package = pod_state.package.clone();

        let client = {
            let provider_state = provider_state.read().await;
            provider_state.client.clone()
        };

        info!("Looking for package: {} in known repositories", &package);
        debug!(
            "Checking if package {} has already been downloaded.",
            package
        );
        if Downloading::package_downloaded(package.clone(), &pod_state.download_directory) {
            info!(
                "Package {} has already been downloaded to {:?}, continuing with installation",
                package, pod_state.download_directory
            );
            return Transition::next(
                self,
                Installing {
                    download_directory: pod_state.download_directory.clone(),
                    parcel_directory: pod_state.parcel_directory.clone(),
                    package: package.clone(),
                },
            );
        }
        let repo = find_repository(client, &package).await;
        return match repo {
            Ok(Some(mut repo)) => {
                // We found a repository providing the package, proceed with download
                // The repository has already downloaded its metadata at this time, as that
                // was used to check whether it provides the package
                info!(
                    "Starting download of package {} from repository {}",
                    &package, &repo
                );
                let download_directory = pod_state.download_directory.clone();

                if !(download_directory.is_dir()) {
                    if let Err(error) = create_download_directory(&download_directory).await {
                        return Transition::Complete(Err(error));
                    }
                };

                let download_result = repo
                    .download_package(&package, download_directory.clone())
                    .await;
                match download_result {
                    Ok(()) => {
                        info!(
                            "Successfully downloaded package {} to {:?}",
                            package,
                            download_directory.clone()
                        );
                        Transition::next(
                            self,
                            Installing {
                                download_directory: pod_state.download_directory.clone(),
                                parcel_directory: pod_state.parcel_directory.clone(),
                                package: package.clone(),
                            },
                        )
                    }
                    Err(e) => {
                        warn!("Download of package {} failed: {}", package, e);
                        Transition::next(
                            self,
                            DownloadingBackoff {
                                package: package.clone(),
                            },
                        )
                    }
                }
            }
            Ok(None) => {
                // No repository was found that provides this package
                let message = format!(
                    "Cannot find package {} in any repository, aborting ..",
                    &package
                );
                error!("{}", &message);
                Transition::next(
                    self,
                    DownloadingBackoff {
                        package: package.clone(),
                    },
                )
            }
            Err(e) => {
                // An error occurred when looking for a repository providing this package
                error!(
                    "Error occurred trying to find package [{}]: [{:?}]",
                    &package, e
                );
                Transition::next(
                    self,
                    DownloadingBackoff {
                        package: package.clone(),
                    },
                )
            }
        };
    }

    async fn status(&self, _pod_state: &mut PodState, _pod: &Pod) -> anyhow::Result<PodStatus> {
        Ok(make_status(Phase::Pending, "Downloading"))
    }
}

async fn create_download_directory(download_directory: &Path) -> anyhow::Result<()> {
    info!("Creating download directory [{:?}].", download_directory);
    create_dir_all(&download_directory).await.with_context(|| {
        format!(
            "Download directory [{}] could not be created.",
            download_directory.to_string_lossy()
        )
    })
}
