use std::collections::{BTreeMap, HashSet};

use serde::Serialize;
use thiserror::Error;

use crate::manifest::{Manifest, ManifestError, Package};

#[derive(Debug, Serialize)]
pub struct InstallPlan {
    pub workspace: String,
    pub target: String,
    pub fingerprint: String,
    pub steps: Vec<PlanStep>,
}

#[derive(Debug, Serialize)]
pub struct PlanStep {
    pub order: usize,
    pub package: String,
    pub version: String,
    pub source: String,
    pub artifact: String,
}

#[derive(Debug, Error)]
pub enum ResolverError {
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error("dependency cycle detected at package '{0}'")]
    DependencyCycle(String),
}

impl InstallPlan {
    pub fn resolve(manifest: &Manifest) -> Result<Self, ResolverError> {
        manifest.validate()?;

        let package_index: BTreeMap<&str, &Package> = manifest
            .packages
            .iter()
            .map(|package| (package.name.as_str(), package))
            .collect();

        let mut visiting = HashSet::new();
        let mut visited = HashSet::new();
        let mut ordered = Vec::new();

        for package in &manifest.packages {
            visit(
                package.name.as_str(),
                &package_index,
                &mut visiting,
                &mut visited,
                &mut ordered,
            )?;
        }

        let steps = ordered
            .iter()
            .enumerate()
            .map(|(index, package)| PlanStep {
                order: index + 1,
                package: package.name.clone(),
                version: package.version.to_string(),
                source: package.source.as_str().to_string(),
                artifact: package.artifact.clone(),
            })
            .collect();

        Ok(Self {
            workspace: manifest.workspace.name.clone(),
            target: manifest.workspace.target.clone(),
            fingerprint: manifest.content_fingerprint(),
            steps,
        })
    }
}

fn visit<'a>(
    package_name: &'a str,
    package_index: &BTreeMap<&'a str, &'a Package>,
    visiting: &mut HashSet<&'a str>,
    visited: &mut HashSet<&'a str>,
    ordered: &mut Vec<&'a Package>,
) -> Result<(), ResolverError> {
    if visited.contains(package_name) {
        return Ok(());
    }
    if !visiting.insert(package_name) {
        return Err(ResolverError::DependencyCycle(package_name.to_string()));
    }

    let package = *package_index
        .get(package_name)
        .expect("manifest validation ensures dependencies exist");

    for dependency in &package.dependencies {
        visit(dependency, package_index, visiting, visited, ordered)?;
    }

    visiting.remove(package_name);
    visited.insert(package_name);
    ordered.push(package);
    Ok(())
}

#[cfg(test)]
mod tests {
    use semver::Version;

    use super::*;
    use crate::manifest::{PackageSource, Toolchain, Workspace};

    fn package(name: &str, dependencies: Vec<&str>) -> Package {
        Package {
            name: name.to_string(),
            version: Version::parse("1.0.0").unwrap(),
            family: "stm32f4".to_string(),
            source: PackageSource::Registry,
            dependencies: dependencies.into_iter().map(String::from).collect(),
            artifact: format!("{name}.zip"),
            sha256: "7c52adf5dd2b1d6a5df9bfe709baedb08f1a65ccfe5f47ccdf67274d87f6d05d".to_string(),
        }
    }

    #[test]
    fn resolves_dependencies_before_dependents() {
        let manifest = Manifest {
            workspace: Workspace {
                name: "motor-control".to_string(),
                target: "stm32f407".to_string(),
                toolchain: Toolchain {
                    compiler: "arm-none-eabi-gcc".to_string(),
                    version: Version::parse("12.3.1").unwrap(),
                },
            },
            packages: vec![
                package("app-template", vec!["stm32f4-hal"]),
                package("stm32f4-hal", vec!["cmsis-core"]),
                package("cmsis-core", vec![]),
            ],
        };

        let plan = InstallPlan::resolve(&manifest).unwrap();
        let names: Vec<_> = plan
            .steps
            .iter()
            .map(|step| step.package.as_str())
            .collect();
        assert_eq!(names, vec!["cmsis-core", "stm32f4-hal", "app-template"]);
    }
}
