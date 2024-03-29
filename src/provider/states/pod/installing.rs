use std::fs;
use std::fs::File;
use std::path::PathBuf;

use flate2::read::GzDecoder;
use kubelet::pod::state::prelude::*;
use kubelet::pod::Pod;
use log::{debug, error, info};
use tar::Archive;

use super::creating_config::CreatingConfig;
use super::setup_failed::SetupFailed;
use crate::provider::error::StackableError;
use crate::provider::repository::package::Package;
use crate::provider::{PodState, ProviderState};

#[derive(Debug, TransitionTo)]
#[transition_to(CreatingConfig, SetupFailed)]
pub struct Installing {
    pub download_directory: PathBuf,
    pub parcel_directory: PathBuf,
    pub package: Package,
}

impl Installing {
    fn package_installed<T: Into<Package>>(&self, package: T) -> bool {
        let package = package.into();

        let target_directory = self.get_target_directory(&package);
        debug!(
            "Checking if package {:?} has already been installed to {:?}",
            package, target_directory
        );
        target_directory.exists()
    }

    fn get_target_directory(&self, package: &Package) -> PathBuf {
        self.parcel_directory.join(package.get_directory_name())
    }

    fn install_package<T: Into<Package>>(&self, package: T) -> Result<(), StackableError> {
        let package: Package = package.into();

        let archive_path = self.download_directory.join(package.get_file_name());
        let tar_gz = File::open(&archive_path)?;
        let tar = GzDecoder::new(tar_gz);
        let mut archive = Archive::new(tar);

        let target_directory = self.get_target_directory(&package);

        info!(
            "Installing package: {:?} from {:?} into {:?}",
            package, archive_path, target_directory
        );
        archive.unpack(target_directory)?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl State<PodState> for Installing {
    async fn next(
        self: Box<Self>,
        _provider_state: SharedState<ProviderState>,
        _pod_state: &mut PodState,
        _pod: Manifest<Pod>,
    ) -> Transition<PodState> {
        let package = self.package.clone();
        let package_name = &package.get_directory_name();
        return if self.package_installed(package.clone()) {
            info!("Package {} has already been installed", package);
            return Transition::next(
                self,
                CreatingConfig {
                    target_directory: None,
                },
            );
        } else {
            info!("Installing package {}", package);
            match self.install_package(package.clone()) {
                Ok(()) => Transition::next(
                    self,
                    CreatingConfig {
                        target_directory: None,
                    },
                ),
                Err(e) => {
                    error!(
                        "Failed to install package [{}] due to: [{:?}]",
                        &package_name, e
                    );
                    // Clean up partially unpacked directory to avoid later iterations assuming
                    // this install attempt was successful because the target directory exists.
                    let installation_directory = self.get_target_directory(&package);
                    debug!(
                        "Cleaning up partial installation by deleting directory [{}]",
                        installation_directory.to_string_lossy()
                    );
                    if let Err(error) = fs::remove_dir_all(&installation_directory) {
                        error!(
                            "Failed to clean up directory [{}] due to {}",
                            installation_directory.to_string_lossy(),
                            error
                        );
                    };
                    Transition::next(
                        self,
                        SetupFailed {
                            message: "PackageInstallationFailed".to_string(),
                        },
                    )
                }
            }
        };
    }

    async fn status(&self, _pod_state: &mut PodState, _pod: &Pod) -> anyhow::Result<PodStatus> {
        Ok(make_status(Phase::Pending, "Installing"))
    }
}
