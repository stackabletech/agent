use std::collections::HashMap;
use std::convert::{TryFrom, TryInto};
use std::hash::{Hash, Hasher};

use kube::api::Meta;
use serde::{Deserialize, Serialize};
use k8s_openapi::url::{ParseError, Url};

use std::path::PathBuf;
use std::fs::File;
use std::io::{Cursor, copy};
use crate::provider::repository::package::Package;
use crate::provider::repository::repository::Repository;
use crate::provider::error::StackableError;
use log::{trace, debug, info, error};
use std::fmt;
use crate::provider::error::StackableError::PackageNotFound;


#[derive(Debug, Clone)]
pub struct StackableRepoProvider {
    base_url: Url,
    pub name: String,
    content: Option<RepositoryContent>,
}

#[derive(Serialize, Deserialize, Debug)]
struct RepoData {
    version: String,
    parcels: HashMap<String, Vec<Product>>,
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
    pub parcels: HashMap<String, HashMap<String, StackablePackage>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct StackablePackage {
    pub product: String,
    pub version: String,
    pub link: String,
    pub hashes: HashMap<String, String>,
}

impl StackableRepoProvider {
    pub fn new(name: String, base_url: String) -> Result<StackableRepoProvider, StackableError> {
        let base_url = Url::parse(&base_url)?;

        Ok(StackableRepoProvider { base_url, name, content: None })
    }

    pub async fn provides_package<T: Into<Package>>(&mut self, package: T) -> Result<bool, StackableError> {
        debug!("Starting metadata refresh for repository of type {} at location {}", "StackableRepo", self.base_url);
        let package = package.into();
        let metadata = self.get_repo_metadata().await?;
        debug!("Repository provides the following products: {:?}", metadata);
        if let Some(product) = metadata.parcels.get(&package.product) {
            return Ok(product.contains_key(&package.version));
        }
        Ok(false)
    }

    async fn get_package(&mut self, package: Package) -> Result<StackablePackage, StackableError> {
        if self.content.is_none() {
            self.get_repo_metadata().await?;
        }
        if let Some(content) = &self.content {
            let parcels = &content.parcels;
            if let Some(product) = parcels.get(&package.product) {
                // product exists in repo
                if let Some(version) = product.get(&package.version) {
                    // found our package
                    return Ok(version.clone());
                }
            };
        }
        Err(PackageNotFound {package})
    }

    pub async fn download_package(&mut self, package: &Package, target_path: PathBuf) -> Result<(), StackableError> {
        if self.content.is_none() {
            let _content = self.get_repo_metadata();
        }

        let stackable_package = self.get_package(package.clone()).await?;
        let download_link = Url::parse(&stackable_package.link)?;
        let mut response = reqwest::get(download_link).await?;

        let mut content =  Cursor::new(response.bytes().await?);

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

        let mut parcels: HashMap<String, HashMap<String, StackablePackage>> = HashMap::new();
        for (product, versions) in repo_data.parcels {
            let mut versionlist = HashMap::new();
            for version in versions {
                versionlist.insert(
                    version.version.clone(),
                    StackablePackage {
                        product: product.clone(),
                        version: version.version,
                        link: self.resolve_url(version.path.clone())?,
                        hashes: version.hashes.clone()
                    },
                );
            }
            parcels.insert(product, versionlist);
        }
        let repo_content: RepositoryContent = RepositoryContent {
            version: repo_data.version,
            parcels,
        };
        self.content = Some(repo_content.clone());
        Ok(repo_content)
    }

    fn resolve_url(&self, path: String) -> Result<String, StackableError> {
        if let Result::Ok(absolute_link) = Url::parse(&path) {
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
        match path {
            Some(gna) => return Ok(StackableRepoProvider { name: Meta::name(value), base_url: Url::parse(gna)?, content: None }),
            None => return Err(StackableError::RepositoryConversionError)
        }
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
    use url::Url;

    #[test]
    fn test_url_functions() {
        assert!(true);
    }
}

