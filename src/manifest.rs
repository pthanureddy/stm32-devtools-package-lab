use std::collections::HashSet;
use std::fs;
use std::path::Path;

use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Manifest {
    pub workspace: Workspace,
    pub packages: Vec<Package>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Workspace {
    pub name: String,
    pub target: String,
    pub toolchain: Toolchain,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Toolchain {
    pub compiler: String,
    pub version: Version,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Package {
    pub name: String,
    pub version: Version,
    pub family: String,
    pub source: PackageSource,
    #[serde(default)]
    pub dependencies: Vec<String>,
    pub artifact: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PackageSource {
    Registry,
    LocalCache,
    Git,
}

impl PackageSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Registry => "registry",
            Self::LocalCache => "local-cache",
            Self::Git => "git",
        }
    }
}

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("manifest file is empty")]
    Empty,
    #[error("unsupported manifest extension '{0}', expected yaml, yml, or json")]
    UnsupportedExtension(String),
    #[error("workspace name must not be empty")]
    EmptyWorkspaceName,
    #[error("target '{0}' must look like an STM32 target, for example stm32f407")]
    InvalidTarget(String),
    #[error("toolchain compiler must not be empty")]
    EmptyCompiler,
    #[error("package list must not be empty")]
    EmptyPackageList,
    #[error("package name must not be empty")]
    EmptyPackageName,
    #[error("duplicate package '{0}'")]
    DuplicatePackage(String),
    #[error("package '{package}' declares unknown dependency '{dependency}'")]
    UnknownDependency { package: String, dependency: String },
    #[error("package '{package}' depends on itself")]
    SelfDependency { package: String },
    #[error("package '{0}' has an invalid STM32 family '{1}'")]
    InvalidFamily(String, String),
    #[error("package '{0}' artifact must not be empty")]
    EmptyArtifact(String),
    #[error("package '{0}' sha256 must be a 64-character lowercase hex digest")]
    InvalidChecksum(String),
}

impl Manifest {
    pub fn from_path(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let content = fs::read_to_string(path)?;
        if content.trim().is_empty() {
            return Err(Box::new(ManifestError::Empty));
        }

        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();

        let manifest = match extension.as_str() {
            "yaml" | "yml" => serde_yaml::from_str(&content)?,
            "json" => serde_json::from_str(&content)?,
            other => {
                return Err(Box::new(ManifestError::UnsupportedExtension(
                    other.to_string(),
                )))
            }
        };

        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.workspace.name.trim().is_empty() {
            return Err(ManifestError::EmptyWorkspaceName);
        }
        if !looks_like_stm32_target(&self.workspace.target) {
            return Err(ManifestError::InvalidTarget(self.workspace.target.clone()));
        }
        if self.workspace.toolchain.compiler.trim().is_empty() {
            return Err(ManifestError::EmptyCompiler);
        }
        if self.packages.is_empty() {
            return Err(ManifestError::EmptyPackageList);
        }

        let mut package_names = HashSet::new();
        for package in &self.packages {
            package.validate_basic()?;
            if !package_names.insert(package.name.clone()) {
                return Err(ManifestError::DuplicatePackage(package.name.clone()));
            }
        }

        for package in &self.packages {
            for dependency in &package.dependencies {
                if dependency == &package.name {
                    return Err(ManifestError::SelfDependency {
                        package: package.name.clone(),
                    });
                }
                if !package_names.contains(dependency) {
                    return Err(ManifestError::UnknownDependency {
                        package: package.name.clone(),
                        dependency: dependency.clone(),
                    });
                }
            }
        }

        Ok(())
    }

    pub fn content_fingerprint(&self) -> String {
        let normalized = serde_json::to_vec(self).unwrap_or_default();
        let digest = Sha256::digest(normalized);
        format!("{digest:x}")
    }
}

impl Package {
    fn validate_basic(&self) -> Result<(), ManifestError> {
        if self.name.trim().is_empty() {
            return Err(ManifestError::EmptyPackageName);
        }
        if !looks_like_stm32_family(&self.family) {
            return Err(ManifestError::InvalidFamily(
                self.name.clone(),
                self.family.clone(),
            ));
        }
        if self.artifact.trim().is_empty() {
            return Err(ManifestError::EmptyArtifact(self.name.clone()));
        }
        if !is_sha256_hex(&self.sha256) {
            return Err(ManifestError::InvalidChecksum(self.name.clone()));
        }
        Ok(())
    }
}

fn looks_like_stm32_target(target: &str) -> bool {
    let lower = target.to_ascii_lowercase();
    lower.starts_with("stm32")
        && lower.len() >= 8
        && lower.chars().all(|c| c.is_ascii_alphanumeric())
}

fn looks_like_stm32_family(family: &str) -> bool {
    let lower = family.to_ascii_lowercase();
    lower.starts_with("stm32")
        && lower.len() >= 7
        && lower.chars().all(|c| c.is_ascii_alphanumeric())
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(name: &str, dependencies: Vec<&str>) -> Package {
        Package {
            name: name.to_string(),
            version: Version::parse("1.0.0").unwrap(),
            family: "stm32f4".to_string(),
            source: PackageSource::Registry,
            dependencies: dependencies.into_iter().map(String::from).collect(),
            artifact: format!("{name}.zip"),
            sha256: "7c52adf5dd2b1d6a5df9bfe709baedb08f1a65ccfe5f47ccdf67274d87f6d05d"
                .to_string(),
        }
    }

    fn manifest(packages: Vec<Package>) -> Manifest {
        Manifest {
            workspace: Workspace {
                name: "demo".to_string(),
                target: "stm32f407".to_string(),
                toolchain: Toolchain {
                    compiler: "arm-none-eabi-gcc".to_string(),
                    version: Version::parse("12.3.1").unwrap(),
                },
            },
            packages,
        }
    }

    #[test]
    fn validates_well_formed_manifest() {
        let manifest = manifest(vec![
            package("cmsis-core", vec![]),
            package("stm32f4-hal", vec!["cmsis-core"]),
        ]);
        assert!(manifest.validate().is_ok());
    }

    #[test]
    fn rejects_unknown_dependency() {
        let manifest = manifest(vec![package("stm32f4-hal", vec!["missing"])]);
        let error = manifest.validate().unwrap_err();
        assert!(matches!(error, ManifestError::UnknownDependency { .. }));
    }

    #[test]
    fn rejects_invalid_checksum() {
        let mut bad = package("cmsis-core", vec![]);
        bad.sha256 = "not-a-checksum".to_string();
        let manifest = manifest(vec![bad]);
        let error = manifest.validate().unwrap_err();
        assert!(matches!(error, ManifestError::InvalidChecksum(_)));
    }
}
