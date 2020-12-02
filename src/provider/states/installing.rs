use std::fs::File;
use std::path::{Path, PathBuf};

use flate2::read::GzDecoder;
use kubelet::pod::Pod;
use kubelet::state::prelude::*;
use kubelet::state::{State, Transition};
use log::{debug, error, info};
use tar::Archive;

use crate::provider::error::StackableError;
use crate::provider::repository::package::Package;
use crate::provider::states::creating_config::CreatingConfig;
use crate::provider::states::setup_failed::SetupFailed;
use crate::provider::PodState;

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

        let package_file_name = self.parcel_directory.join(package.get_directory_name());
        debug!(
            "Checking if package {:?} has already been installed to {:?}",
            package, package_file_name
        );
        Path::new(&package_file_name).exists()
    }

    fn get_target_directory(&self, package: Package) -> PathBuf {
        self.parcel_directory.join(package.get_directory_name())
    }

    fn install_package<T: Into<Package>>(&self, package: T) -> Result<(), StackableError> {
        let package: Package = package.into();

        let archive_path = self.download_directory.join(package.get_file_name());
        let tar_gz = File::open(&archive_path)?;
        let tar = GzDecoder::new(tar_gz);
        let mut archive = Archive::new(tar);

        let target_directory = self.get_target_directory(package.clone());

        info!(
            "Installing package: {:?} from {:?} into {:?}",
            package, archive_path, target_directory
        );
        archive.unpack(self.parcel_directory.join(package.get_directory_name()))?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl State<PodState> for Installing {
    async fn next(self: Box<Self>, _pod_state: &mut PodState, _pod: &Pod) -> Transition<PodState> {
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
            match self.install_package(package) {
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

    async fn json_status(
        &self,
        _pod_state: &mut PodState,
        _pod: &Pod,
    ) -> anyhow::Result<serde_json::Value> {
        make_status(Phase::Pending, &"Installing")
    }
}
