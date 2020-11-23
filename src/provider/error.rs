use thiserror::Error;
use k8s_openapi::url;
use crate::provider::repository::package::Package;
use handlebars::{RenderError, TemplateError};


#[derive(Error, Debug)]
pub enum StackableError {
    #[error(transparent)]
    Parse(#[from] url::ParseError),
    #[error(transparent)]
    Reqwest(#[from] reqwest::Error),
    #[error(transparent)]
    IO(#[from] std::io::Error),
    #[error("unable to create repository from received repo object")]
    RepositoryConversionError,
    #[error("error parsing package from containerimage string, has to be in the form of: \"repositoryname/package:version\"")]
    PackageParseError,
    #[error("Invalid content in pod object: {msg}")]
    PodValidationError{msg: String},
    #[error(transparent)]
    Kube(#[from] kube::Error),
    #[error(transparent)]
    TemplateRenderError(#[from] RenderError),
    #[error(transparent)]
    TemplateError(#[from] TemplateError),
    #[error("A required CRD has not been registered: {missing_crds:?}")]
    CrdMissing{missing_crds: Vec<String>},
    #[error("Package {package} not found in repository")]
    PackageNotFound{package: Package},
    #[error("{msg}")]
    RuntimeError{msg: String}
}