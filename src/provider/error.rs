use handlebars::{RenderError, TemplateError};
use k8s_openapi::url;
use thiserror::Error;

use crate::provider::repository::package::Package;
use std::ffi::OsString;

#[derive(Error, Debug)]
pub enum StackableError {
    #[error(transparent)]
    Parse(#[from] url::ParseError),
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("unable to create repository from received repo object")]
    RepositoryConversionError,
    #[error("Invalid content in pod object: {msg}")]
    PodValidationError { msg: String },
    #[error("Kubernetes reported error: {source}")]
    KubeError {
        #[from]
        source: kube::Error,
    },
    #[error(transparent)]
    TemplateRenderError(#[from] RenderError),
    #[error(transparent)]
    TemplateError(#[from] TemplateError),
    #[error("A required CRD has not been registered: {missing_crds:?}")]
    CrdMissing { missing_crds: Vec<String> },
    #[error("Package {package} not found in repository")]
    PackageNotFound { package: Package },
    #[error("{msg}")]
    RuntimeError { msg: String },
    #[error("Unable to parse data for {target} from non-UTF8 String: {original:?}")]
    DirectoryParseError { target: String, original: OsString },
    #[error("An error ocurred trying to write Config Map {config_map} to file {target_file}")]
    ConfigFileWriteError {
        target_file: String,
        config_map: String,
    },
    #[error(
        "The following config maps were specified in a pod but not found: {missing_config_maps:?}"
    )]
    MissingConfigMapsError { missing_config_maps: Vec<String> },
    #[error("Object is missing key: {key}")]
    MissingObjectKey { key: &'static str },
}
