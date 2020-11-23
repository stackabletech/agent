use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use crate::provider::repository::package::Package;
use kube_derive::CustomResource;

#[derive(CustomResource, Serialize, Deserialize, Default, Clone, Debug)]
#[kube(
kind = "Repository",
group = "stable.stackable.de",
version = "v1",
namespaced
)]
pub struct RepositorySpec {
    pub repo_type: RepoType,
    pub properties: HashMap<String, String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum RepoType {
    StackableRepo,
}

impl Default for RepoType {
    fn default() -> Self {
        RepoType::StackableRepo
    }
}
