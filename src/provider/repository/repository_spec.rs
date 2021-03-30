use std::collections::HashMap;

use kube_derive::CustomResource;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(CustomResource, Serialize, Deserialize, Default, Clone, Debug, JsonSchema)]
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

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub enum RepoType {
    StackableRepo,
}

impl Default for RepoType {
    fn default() -> Self {
        RepoType::StackableRepo
    }
}
