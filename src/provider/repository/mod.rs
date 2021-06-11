use std::convert::TryFrom;

use kube::api::ListParams;
use kube::{Api, Client};
use log::{debug, trace};

use crate::provider::error::StackableError;
use crate::provider::repository::package::Package;
use crate::provider::repository::repository_spec::Repository;
use crate::provider::repository::stackablerepository::StackableRepoProvider;

pub mod package;
pub mod repository_spec;
pub mod stackablerepository;

pub async fn find_repository(
    client: Client,
    package: &Package,
    repository_reference: Option<String>,
) -> Result<Option<StackableRepoProvider>, StackableError> {
    let repositories: Api<Repository> = Api::namespaced(client.clone(), "default");
    if let Some(repository_name) = repository_reference {
        // A repository name was provided, just check that exact repository for the package
        let repo = repositories.get(&repository_name).await?;
        let mut repo = StackableRepoProvider::try_from(&repo)?;
        if repo.provides_package(package.clone()).await? {
            return Ok(Some(repo));
        }
        return Ok(None);
    } else {
        // No repository name was provided, retrieve all repositories from the orchestrator/apiserver
        // and check which one provides the package
        let list_params = ListParams::default();
        let repos = repositories.list(&list_params).await?;
        for repository in repos.iter() {
            debug!("got repo definition: [{:?}]", repository);
            // Convert repository to object implementing our trait
            let mut repo = StackableRepoProvider::try_from(repository)?;
            trace!("converted to stackable repo: {:?}", repository);
            if repo.provides_package(package.clone()).await? {
                debug!("Found package [{}] in repository [{}]", &package, repo);
                return Ok(Some(repo));
            } else {
                debug!(
                    "Package [{}] not provided by repository [{}]",
                    &package, repo
                );
            }
        }
    }
    Ok(None)
}
