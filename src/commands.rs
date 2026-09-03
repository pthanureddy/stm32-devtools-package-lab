use std::path::Path;

use crate::manifest::Manifest;
use crate::repack::{default_sidecar_path, repack_distribution, RepackRequest};
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

pub fn repack_command(
    base: &Path,
    overlay: &Path,
    config: &Path,
    output: &Path,
    manifest: Option<&Path>,
    force: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let request = RepackRequest {
        base_dir: base.to_path_buf(),
        overlay_dir: overlay.to_path_buf(),
        config_path: config.to_path_buf(),
        output_path: output.to_path_buf(),
        manifest_path: manifest
            .map(Path::to_path_buf)
            .unwrap_or_else(|| default_sidecar_path(output)),
        force,
    };
    let result = repack_distribution(&request)?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}
