use kubelet::state::{State, Transition};
use kubelet::pod::Pod;
use kubelet::state::prelude::*;
use crate::provider::PodState;
use crate::provider::states::running::Running;
use crate::provider::states::failed::Failed;
use crate::provider::states::install_package::Installing;
use crate::provider::states::setup_failed::SetupFailed;
use crate::provider::error::StackableError::PodValidationError;
use crate::fail_fatal;
use kube::api::Meta;
use k8s_openapi::api::core::v1::PodSpec;
use crate::provider::repository::package::Package;
use crate::provider::error::StackableError;
use kubelet::container::Container;
use std::convert::TryFrom;
use log::{debug, info, warn, error};
use crate::provider::repository::find_repository;
use crate::provider::states::download_package_backoff::DownloadingBackoff;
use std::path::{Path, PathBuf};

#[derive(Default, Debug, TransitionTo)]
#[transition_to(Installing, DownloadingBackoff)]
pub struct Downloading;

impl Downloading {
    fn package_downloaded<T: Into<Package>>(&self, package: T, download_directory: PathBuf) -> bool {
        let package = package.into();
        let package_file_name = download_directory.join(package.get_file_name());
        debug!("Checking if package {} has already been downloaded to {:?}", package, package_file_name);
        Path::new(&package_file_name).exists()
    }
}

#[async_trait::async_trait]
impl State<PodState> for Downloading {
    async fn next(self: Box<Self>, pod_state: &mut PodState, _pod: &Pod) -> Transition<PodState> {
        let package = pod_state.package.clone();

        info!("Looking for package: {} in known repositories", &package);
        debug!("Checking if package {} has already been downloaded.", package);
        if self.package_downloaded(package.clone(), pod_state.download_directory.clone()) {
            info!("Package {} has already been downloaded to {:?}, continuing with installation", package, pod_state.download_directory);
            return Transition::next(self, Installing {
                download_directory: pod_state.download_directory.clone(),
                parcel_directory: pod_state.parcel_directory.clone(),
                package: package.clone(),
            });
        }
        let repo = find_repository(pod_state.client.clone(), &package, None).await;
        match repo {
            Ok(Some(mut repo)) => {
                // We found a repository providing the package, proceed with download
                // The repository has already downloaded its metadata it this time, as that
                // was used to check whether it provides the package
                info!("Starting download of package {} from repository {}", &package, &repo);
                let download_directory = pod_state.download_directory.clone();
                let download_result = repo.download_package(&package, download_directory.clone()).await;
                match download_result {
                    Ok(()) => {
                        info!("Successfully downloaded package {} to {:?}", package, download_directory.clone());
                        return Transition::next(self, Installing {
                            download_directory: pod_state.download_directory.clone(),
                            parcel_directory: pod_state.parcel_directory.clone(),
                            package: package.clone(),
                        });
                    }
                    Err(e) => {
                        warn!("Download of package {} failed: {}", package, e);
                        return Transition::next(self, DownloadingBackoff { package: package.clone() });
                    }
                }
            }
            Ok(None) => {
                // No repository was found that provides this package
                let message = format!("Cannot find package {} in any repository, aborting ..", &package);
                error!("{}", &message);
                return Transition::next(self, DownloadingBackoff { package: package.clone() });
            }
            Err(e) => {
                // An error occurred when looking for a repository providing this package
                let message = format!("Error occurred trying to find package {}: {:?}", &package, e);
                error!("{}", &message);
                return Transition::next(self, DownloadingBackoff { package: package.clone() });
            }
        }
    }

    async fn json_status(
        &self,
        _pod_state: &mut PodState,
        _pod: &Pod,
    ) -> anyhow::Result<serde_json::Value> {
        make_status(Phase::Pending, &"status:initializing")
    }
}

