use std::convert::TryFrom;
use std::fmt;

use anyhow::{anyhow, Result};
use oci_distribution::Reference;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Package {
    pub product: String,
    pub version: String,
}

impl Package {
    /// Derive a standardized archive name to use when downloading this package into the
    /// _download folder.
    /// This helps with not downloading the same version of a product twice simply due to
    /// different archive names.
    /// Currently this assumes all archives to be in .tar.gz format, we might revisit this at
    /// a later stage.
    pub fn get_file_name(&self) -> String {
        format!("{}.tar.gz", self.get_directory_name())
    }

    /// Derive a standardized name for the folder that this package should be installed to.
    /// This helps avoiding duplicate binary installations due to different folder names.
    pub fn get_directory_name(&self) -> String {
        format!("{}-{}", self.product, self.version)
    }
}

impl TryFrom<Reference> for Package {
    type Error = anyhow::Error;

    // Converts from an oci reference to a package representation
    // The oci tag (anything after the \":\" in the string) is used as
    // version by this code and needs to be present
    fn try_from(value: Reference) -> Result<Self> {
        let repository = value.repository();
        let tag = value.tag().ok_or(anyhow!("Tag is required."))?;

        Ok(Package {
            product: String::from(repository),
            version: String::from(tag),
        })
    }
}

impl fmt::Display for Package {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.product, self.version)
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn try_from_complete_reference() {
        let reference = Reference::try_from("kafka:2.7").expect("Reference cannot be parsed.");

        let maybe_package = Package::try_from(reference);

        if let Ok(package) = maybe_package {
            assert_eq!("kafka", package.product);
            assert_eq!("2.7", package.version);
        } else {
            panic!("Package expected but got {:?}", maybe_package);
        }
    }

    #[test]
    fn try_from_reference_without_tag() {
        let reference = Reference::try_from("kafka").expect("Reference cannot be parsed.");

        let maybe_package = Package::try_from(reference);

        if let Err(error) = maybe_package {
            assert_eq!("Tag is required.", error.to_string());
        } else {
            panic!("Error expected but got {:?}", maybe_package);
        }
    }
}
