use std::convert::TryFrom;
use std::fmt;

use oci_distribution::Reference;
use serde::{Deserialize, Serialize};

use crate::provider::error::StackableError;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Package {
    pub product: String,
    pub version: String,
}

impl Package {
    pub fn get_file_name(&self) -> String {
        format!("{}.tar.gz", self.get_directory_name())
    }

    pub fn get_directory_name(&self) -> String {
        format!("{}-{}", self.product, self.version)
    }
}

impl TryFrom<Reference> for Package {
    type Error = StackableError;

    fn try_from(value: Reference) -> Result<Self, Self::Error> {
        Ok(Package {
            product: String::from(value.repository()),
            version: String::from(value.tag().unwrap()),
        })
    }
}

impl fmt::Display for Package {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.product, self.version)
    }
}

