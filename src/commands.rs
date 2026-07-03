use std::path::Path;

use crate::manifest::Manifest;
use crate::resolver::InstallPlan;

pub fn validate_manifest(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let manifest = Manifest::from_path(path)?;
    manifest.validate()?;
    println!(
        "manifest '{}' is valid for target {} with {} package(s)",
        manifest.workspace.name,
        manifest.workspace.target,
        manifest.packages.len()
    );
    Ok(())
}

pub fn plan_manifest(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let manifest = Manifest::from_path(path)?;
    let plan = InstallPlan::resolve(&manifest)?;
    println!("{}", serde_json::to_string_pretty(&plan)?);
    Ok(())
}

pub fn inspect_manifest(path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let manifest = Manifest::from_path(path)?;
    manifest.validate()?;
    println!("{}", serde_json::to_string_pretty(&manifest)?);
    Ok(())
}

