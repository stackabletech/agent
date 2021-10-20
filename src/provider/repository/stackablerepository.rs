use std::collections::HashMap;
use std::convert::TryFrom;
use std::fmt;
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::io::{copy, Cursor, Write};
use std::path::PathBuf;

use crate::provider::error::StackableError;
use crate::provider::error::StackableError::{PackageDownloadError, PackageNotFound};
use crate::provider::repository::package::Package;
use crate::provider::repository::repository_spec::Repository;
use kube::api::Meta;
use log::{debug, trace, warn};
use reqwest::header::{ACCEPT, CONTENT_TYPE};
use reqwest::{Client, StatusCode};
use serde::{Deserialize, Serialize};
use url::Url;

// These are the default content_types that we have seen in the wild
// of these only 'application/gzip' is valid according to
// https://www.iana.org/assignments/media-types/media-types.xhtml but our own
// Nexus uses the other two, so we cannot really complain
const DEFAULT_ALLOWED_CONTENT_TYPES: &[&str] = &[
    "application/gzip",
    "application/tgz",
    "application/x-gzip",
    "application/x-tgz",
];

#[derive(Debug, Clone)]
pub struct StackableRepoProvider {
    metadata_url: Url,
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
    pub fn new(name: &str, base_url: &Url) -> Result<StackableRepoProvider, StackableError> {
        let mut metadata_url = base_url.to_owned();

        metadata_url
            .path_segments_mut()
            .map_err(|_| StackableError::RepositoryConversionError)?
            .pop_if_empty()
            .push("metadata.json");

        Ok(StackableRepoProvider {
            metadata_url,
            name: String::from(name),
            content: None,
        })
    }

    pub async fn provides_package<T: Into<Package>>(
        &mut self,
        package: T,
    ) -> Result<bool, StackableError> {
        debug!(
            "Starting metadata refresh for repository of type {} at location {}",
            "StackableRepo", self.metadata_url
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

        let client = Client::builder()
            .build()
            .map_err(|error| PackageDownloadError {
                package: package.clone(),
                download_link: download_link.clone(),
                errormessage: format!("Unable to create http client: [{}]", error),
            })?;

        // We set the ACCEPT header field on our request which states that the only content type
        // we are willing to accept is 'application/gzip'
        // If the webserver is unable to provide this content type to us it _SHOULD_ respond with a
        // 406 response code, but it seems we can't rely on that.
        // For more details see: https://www.w3.org/Protocols/rfc2616/rfc2616-sec14.html#sec14.1
        let response = match client
            .get(download_link.clone())
            .header(ACCEPT, "application/gzip")
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => {
                // The request was successful, but just to be safe we'll still check the content_type, 
                // since the webserver is free to ignore the requested content_type
                if let Some(content_type) = response.headers().get(CONTENT_TYPE) {
                    let content_type = content_type.to_str().map_err(|error| PackageDownloadError {
                        package: package.clone(),
                        download_link: download_link.clone(),
                        errormessage: format!("Got content_type with non-ascii characters from webserver: [{}]", error),
                    })?;

                    if DEFAULT_ALLOWED_CONTENT_TYPES.contains(&content_type) {
                        Ok(response)
                    } else {
                        // If we get a known wrong content type we'll abort
                        Err(PackageDownloadError {
                            package: package.clone(),
                            download_link,
                            errormessage: format!(
                                "Got wrong 'content_type' header [{:?}] in response from webserver.",
                                content_type
                            ),
                        })
                    }
                } else {
                    // If we get no content_type (not sure if this is even legal) we'll soldier on and hope for the best
                    debug!("Response had no 'content_type' header set, we'll give the sender the benefit of the doubt and try processing this anyway.");
                    Ok(response)
                }
            }
            Ok(response) if response.status() == StatusCode::NOT_ACCEPTABLE =>
                Err(PackageDownloadError {
                    package: package.clone(),
                    download_link,
                    errormessage: "Got response code 406 from webserver - Unable to negotiate content type, this is probably due to content encoding settings on the webserver.".to_string(),
                }),
            Ok(response) => Err(PackageDownloadError {
                package: package.clone(),
                download_link,
                errormessage: format!(
                    "Got non-success response [{}] from webserver!",
                    response.status()
                ),
            }),
            Err(error) => Err(PackageDownloadError {
                package: package.clone(),
                download_link,
                errormessage: format!("{}", error),
            }),
        }?;

        // All error cases return above, so we can safely assume that this is a valid download at
        // this point
        let mut content = Cursor::new(response.bytes().await?);

        let mut out = File::create(target_path.join(package.get_file_name()))?;
        copy(&mut content, &mut out)?;
        out.flush()?;
        Ok(())
    }

    async fn get_repo_metadata(&mut self) -> Result<RepositoryContent, StackableError> {
        trace!("entering get_repo_metadata");

        debug!("Retrieving repository metadata from {}", self.metadata_url);

        let repo_data = match reqwest::get(self.metadata_url.clone()).await {
            Ok(repo_data) => repo_data,
            Err(error) => {
                warn!(
                    "Failed to retrieve metadata from {} due to {:?}",
                    self.metadata_url, error
                );
                return Err(error.into());
            }
        };
        let repo_data = match repo_data.json::<RepoData>().await {
            Ok(parsed_data) => parsed_data,
            Err(error) => {
                warn!(
                    "Error parsing metadata from repository {}: {:?}",
                    self.name, error
                );
                return Err(error.into());
            }
        };

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
        let resolved_path = self.metadata_url.join(&path)?;
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
        let name = Meta::name(value);

        let base_url = value
            .spec
            .properties
            .get("url")
            .and_then(|url| Url::parse(url).ok())
            .ok_or(StackableError::RepositoryConversionError)?;

        let stackable_repo_provider = StackableRepoProvider::new(&name, &base_url)?;

        Ok(stackable_repo_provider)
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
    use super::*;

    use crate::provider::repository::repository_spec::RepositorySpec;

    #[test]
    fn stackable_repo_provider_should_be_created_from_a_valid_url_with_a_trailing_slash() {
        let actual =
            StackableRepoProvider::new("test", &Url::parse("http://localhost:8000/repo/").unwrap())
                .unwrap();

        assert_eq!(
            Url::parse("http://localhost:8000/repo/metadata.json").unwrap(),
            actual.metadata_url
        );
        assert_eq!(String::from("test"), actual.name);
        assert!(actual.content.is_none());
    }

    #[test]
    fn stackable_repo_provider_should_be_created_from_a_valid_url_without_a_trailing_slash() {
        let actual =
            StackableRepoProvider::new("test", &Url::parse("http://localhost:8000/repo").unwrap())
                .unwrap();

        assert_eq!(
            Url::parse("http://localhost:8000/repo/metadata.json").unwrap(),
            actual.metadata_url
        );
        assert_eq!(String::from("test"), actual.name);
        assert!(actual.content.is_none());
    }

    #[test]
    fn stackable_repo_provider_should_not_be_created_from_an_url_which_cannot_be_a_base() {
        assert!(StackableRepoProvider::new(
            "test",
            &Url::parse("mailto:info@stackable.de").unwrap()
        )
        .is_err());
    }

    #[test]
    fn test_url_functions() {
        let repo =
            StackableRepoProvider::new("test", &Url::parse("http://localhost:8000").unwrap())
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
            converted_repo.metadata_url.as_str(),
            "http://monitoring.stackable.demo:8000/metadata.json"
        );
    }
}
