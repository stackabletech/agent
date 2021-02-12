use std::collections::HashMap;
use std::convert::TryFrom;
use std::fmt;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{copy, Cursor};
use std::path::PathBuf;

use kube::api::Meta;
use log::{debug, trace};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::provider::error::StackableError;
use crate::provider::error::StackableError::PackageNotFound;
use crate::provider::repository::package::Package;
use crate::provider::repository::repository_spec::Repository;

#[derive(Debug, Clone)]
pub struct StackableRepoProvider {
    base_url: Url,
    pub name: String,
    content: Option<RepositoryContent>,
}

#[derive(Serialize, Deserialize, Debug)]
struct RepoData {
    version: String,
    packages: HashMap<String, Vec<Product>>,
}

#[derive(Serialize, Deserialize, Debug)]
struct Product {
    version: String,
    path: String,
    hashes: HashMap<String, String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct RepositoryContent {
    pub version: String,
    pub packages: HashMap<String, HashMap<String, StackablePackage>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct StackablePackage {
    pub product: String,
    pub version: String,
    pub link: String,
    pub hashes: HashMap<String, String>,
}

impl StackableRepoProvider {
    // This is only used in a test case and hence warned about as dead code
    #[allow(dead_code)]
    pub fn new(name: String, base_url: String) -> Result<StackableRepoProvider, StackableError> {
        let base_url = Url::parse(&base_url)?;

        Ok(StackableRepoProvider {
            base_url,
            name,
            content: None,
        })
    }

    pub async fn provides_package<T: Into<Package>>(
        &mut self,
        package: T,
    ) -> Result<bool, StackableError> {
        debug!(
            "Starting metadata refresh for repository of type {} at location {}",
            "StackableRepo", self.base_url
        );
        let package = package.into();
        let metadata = self.get_repo_metadata().await?;
        debug!("Repository provides the following products: {:?}", metadata);
        if let Some(product) = metadata.packages.get(&package.product) {
            return Ok(product.contains_key(&package.version));
        }
        Ok(false)
    }

    async fn get_package(&mut self, package: Package) -> Result<StackablePackage, StackableError> {
        if self.content.is_none() {
            self.get_repo_metadata().await?;
        }
        if let Some(content) = &self.content {
            let parcels = &content.packages;
            if let Some(product) = parcels.get(&package.product) {
                // product exists in repo
                if let Some(version) = product.get(&package.version) {
                    // found our package
                    return Ok(version.clone());
                }
            };
        }
        Err(PackageNotFound { package })
    }

    pub async fn download_package(
        &mut self,
        package: &Package,
        target_path: PathBuf,
    ) -> Result<(), StackableError> {
        if self.content.is_none() {
            let _content = self.get_repo_metadata();
        }

        let stackable_package = self.get_package(package.clone()).await?;
        let download_link = Url::parse(&stackable_package.link)?;
        let response = reqwest::get(download_link).await?;

        let mut content = Cursor::new(response.bytes().await?);

        let mut out = File::create(target_path.join(package.get_file_name()))?;
        copy(&mut content, &mut out)?;
        Ok(())
    }

    // TODO: implement caching based on version of metadata
    async fn get_repo_metadata(&mut self) -> Result<RepositoryContent, StackableError> {
        trace!("entering get_repo_metadata");
        let mut metadata_url = self.base_url.clone();

        // TODO: add error propagation
        // path_segments_mut returns () in an error case, not sure how to handle this
        metadata_url
            .path_segments_mut()
            .expect("")
            .push("metadata.json");

        debug!("Retrieving repository metadata from {}", metadata_url);

        let repo_data = reqwest::get(metadata_url).await?.json::<RepoData>().await?;

        debug!("Got repository metadata: {:?}", repo_data);

        let mut packages: HashMap<String, HashMap<String, StackablePackage>> = HashMap::new();
        for (product, versions) in repo_data.packages {
            let mut versionlist = HashMap::new();
            for version in versions {
                versionlist.insert(
                    version.version.clone(),
                    StackablePackage {
                        product: product.clone(),
                        version: version.version,
                        link: self.resolve_url(version.path.clone())?,
                        hashes: version.hashes.clone(),
                    },
                );
            }
            packages.insert(product, versionlist);
        }
        let repo_content: RepositoryContent = RepositoryContent {
            version: repo_data.version,
            packages,
        };
        self.content = Some(repo_content.clone());
        Ok(repo_content)
    }

    /// Resolves relative paths that are defined for elements in this repository against
    /// the repo's base URL.
    /// Unless the element has an absolute URL defined, in this case the base URL is ignored
    /// an the absolute URL returned unchanged.
    ///
    /// Public for testing
    pub fn resolve_url(&self, path: String) -> Result<String, StackableError> {
        if Url::parse(&path).is_ok() {
            // The URL defined for this element is an absolute URL, so we won't
            // resolve that agains the base url of the repository but simply
            // return it unchanged
            return Ok(path);
        }
        let resolved_path = self.base_url.join(&path)?;
        Ok(resolved_path.as_str().to_string())
    }
}

impl fmt::Display for StackableRepoProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)
    }
}

impl TryFrom<&Repository> for StackableRepoProvider {
    type Error = StackableError;

    fn try_from(value: &Repository) -> Result<Self, Self::Error> {
        let properties: HashMap<String, String> = value.clone().spec.properties;
        let path = properties.get("url");
        if let Some(valid_path) = path {
            return Ok(StackableRepoProvider {
                name: Meta::name(value),
                base_url: Url::parse(valid_path)?,
                content: None,
            });
        }
        Err(StackableError::RepositoryConversionError)
    }
}

impl Eq for StackableRepoProvider {}

impl PartialEq for StackableRepoProvider {
    fn eq(&self, other: &Self) -> bool {
        self.name.eq(&other.name)
    }
}

impl Hash for StackableRepoProvider {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.name.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use crate::provider::repository::repository_spec::{Repository, RepositorySpec};
    use crate::provider::repository::stackablerepository::StackableRepoProvider;
    use std::collections::HashMap;
    use std::convert::TryFrom;

    #[test]
    fn test_url_functions() {
        let repo =
            StackableRepoProvider::new(String::from("test"), String::from("http://localhost:8000"))
                .unwrap();

        // Check that a relative URL is correctly resolved against the repo's baseurl
        assert_eq!(
            repo.resolve_url(String::from("test")).unwrap(),
            "http://localhost:8000/test"
        );

        // Test that an absolute URL is correctly returned without change
        assert_eq!(
            repo.resolve_url(String::from("http://test.com/test"))
                .unwrap(),
            "http://test.com/test"
        );
    }

    #[test]
    fn test_repository_try_from() {
        let mut props = HashMap::new();
        props.insert(
            String::from("url"),
            String::from("http://monitoring.stackable.demo:8000"),
        );
        let test_repo_crd = Repository::new(
            "test",
            RepositorySpec {
                repo_type: Default::default(),
                properties: props,
            },
        );
        let converted_repo = StackableRepoProvider::try_from(&test_repo_crd).unwrap();
        assert_eq!(converted_repo.name, "test");
        assert_eq!(
            converted_repo.base_url.as_str(),
            "http://monitoring.stackable.demo:8000/"
        );
    }
}
