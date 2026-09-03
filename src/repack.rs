//! Deterministic, host-side repacking of an existing distribution with a
//! product overlay.
//!
//! The module deliberately operates on ordinary files only. It rejects links,
//! special files, unsafe archive paths, overlapping inputs, and outputs placed
//! inside either input tree. Payload bytes are staged before archiving so the
//! per-file digests describe the exact bytes written to the archive.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use flate2::{Compression, GzBuilder};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tar::{Builder as TarBuilder, Header};
use tempfile::{Builder as TempBuilder, NamedTempFile, TempDir};
use thiserror::Error;
use walkdir::WalkDir;

const CONFIG_SCHEMA_VERSION: u32 = 1;
const MANIFEST_SCHEMA_VERSION: u32 = 1;
const EMBEDDED_MANIFEST_PATH: &str = "PACKAGE-MANIFEST.json";
const COPY_BUFFER_SIZE: usize = 64 * 1024;

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct RepackConfig {
    pub schema_version: u32,
    pub product: ProductIdentity,
    pub base_distribution: DistributionIdentity,
    pub target: String,
    #[serde(default)]
    pub executable_paths: Vec<String>,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProductIdentity {
    pub name: String,
    pub release: Version,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DistributionIdentity {
    pub name: String,
    pub release: Version,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepackManifest {
    pub schema_version: u32,
    pub archive_format: String,
    pub generated_by: String,
    pub product: ProductIdentity,
    pub base_distribution: DistributionIdentity,
    pub target: String,
    pub configuration_sha256: String,
    pub labels: BTreeMap<String, String>,
    pub payload: PayloadSummary,
    pub files: Vec<FileRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PayloadSummary {
    pub file_count: usize,
    pub total_size: u64,
    /// Digest of the canonical, sorted file-record stream. Each record binds
    /// its path, source layer, size, content digest, and archive mode.
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileRecord {
    pub path: String,
    pub layer: Layer,
    pub size: u64,
    pub sha256: String,
    pub mode: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Layer {
    Base,
    Overlay,
}

impl Layer {
    fn as_str(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Overlay => "overlay",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RepackRequest {
    pub base_dir: PathBuf,
    pub overlay_dir: PathBuf,
    pub config_path: PathBuf,
    pub output_path: PathBuf,
    pub manifest_path: PathBuf,
    pub force: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RepackResult {
    pub archive: PathBuf,
    pub manifest: PathBuf,
    pub archive_sha256: String,
    pub payload_sha256: String,
    pub file_count: usize,
    pub total_size: u64,
}

#[derive(Debug, Error)]
pub enum RepackError {
    #[error("failed to {operation} '{}': {source}", path.display())]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("unsupported configuration extension '{0}', expected yaml, yml, or json")]
    UnsupportedConfigExtension(String),
    #[error("invalid YAML configuration: {0}")]
    InvalidYaml(#[from] serde_yaml::Error),
    #[error("invalid JSON configuration: {0}")]
    InvalidJson(#[from] serde_json::Error),
    #[error("invalid repack configuration: {0}")]
    InvalidConfig(String),
    #[error("input '{path}' is not a directory")]
    InputNotDirectory { path: PathBuf },
    #[error("input '{path}' is not a regular file")]
    InputNotFile { path: PathBuf },
    #[error(
        "base and overlay directory trees must not overlap: '{}' and '{}'",
        base.display(),
        overlay.display()
    )]
    OverlappingInputs { base: PathBuf, overlay: PathBuf },
    #[error("unsafe path '{path}': {reason}")]
    UnsafePath { path: String, reason: &'static str },
    #[error("symbolic links are not packaged: '{path}'")]
    Symlink { path: String },
    #[error("unsupported file type in input tree: '{path}'")]
    UnsupportedFileType { path: String },
    #[error("payload path collision between file '{file}' and descendant '{descendant}'")]
    PathCollision { file: String, descendant: String },
    #[error("payload path is reserved for the embedded release manifest: '{0}'")]
    ReservedManifestPath(String),
    #[error("configured executable path is not present in the merged payload: '{0}'")]
    MissingExecutable(String),
    #[error("output path must not be inside an input tree: '{}'", path.display())]
    OutputInsideInput { path: PathBuf },
    #[error("archive and sidecar manifest paths must be different")]
    DuplicateOutputPaths,
    #[error(
        "output path must not replace the repack configuration: '{}'",
        path.display()
    )]
    OutputOverlapsConfig { path: PathBuf },
    #[error("output already exists (pass --force to replace it): '{}'", path.display())]
    OutputExists { path: PathBuf },
    #[error("archive entry exceeds addressable memory on this host: '{0}'")]
    EntryTooLarge(String),
    #[error("failed to persist temporary output as '{}': {source}", path.display())]
    Persist {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

#[derive(Debug, Clone)]
struct SourceFile {
    source_path: PathBuf,
    layer: Layer,
}

#[derive(Debug)]
struct PreparedFile {
    staged_path: PathBuf,
    record: FileRecord,
    archive_mode: u32,
}

impl RepackConfig {
    pub fn from_path(path: &Path) -> Result<Self, RepackError> {
        let content = fs::read_to_string(path).map_err(|source| io_error("read", path, source))?;
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();

        let config = match extension.as_str() {
            "yaml" | "yml" => serde_yaml::from_str(&content)?,
            "json" => serde_json::from_str(&content)?,
            other => return Err(RepackError::UnsupportedConfigExtension(other.to_string())),
        };
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), RepackError> {
        if self.schema_version != CONFIG_SCHEMA_VERSION {
            return Err(RepackError::InvalidConfig(format!(
                "unsupported schema_version {}, expected {}",
                self.schema_version, CONFIG_SCHEMA_VERSION
            )));
        }
        validate_identifier("product.name", &self.product.name)?;
        validate_identifier("base_distribution.name", &self.base_distribution.name)?;
        validate_metadata_value("target", &self.target)?;

        let mut executables = BTreeSet::new();
        for path in &self.executable_paths {
            let normalized = validate_relative_path(path)?;
            if !executables.insert(normalized.clone()) {
                return Err(RepackError::InvalidConfig(format!(
                    "duplicate executable path '{normalized}'"
                )));
            }
        }

        for (key, value) in &self.labels {
            validate_metadata_value("label key", key)?;
            validate_metadata_value("label value", value)?;
        }
        Ok(())
    }

    fn canonical_sha256(&self) -> Result<String, RepackError> {
        let mut canonical = self.clone();
        canonical.executable_paths = self
            .executable_paths
            .iter()
            .map(|path| validate_relative_path(path))
            .collect::<Result<_, _>>()?;
        canonical.executable_paths.sort();
        let bytes = serde_json::to_vec(&canonical)?;
        Ok(sha256_bytes(&bytes))
    }
}

/// Merge `overlay_dir` over `base_dir`, emit a deterministic tar.gz archive,
/// and write the same deterministic manifest both inside the archive and as a
/// sidecar JSON file.
pub fn repack_distribution(request: &RepackRequest) -> Result<RepackResult, RepackError> {
    let config_path = canonical_input_file(&request.config_path)?;
    let config = RepackConfig::from_path(&config_path)?;
    config.validate()?;

    let base = canonical_input_directory(&request.base_dir)?;
    let overlay = canonical_input_directory(&request.overlay_dir)?;
    if base == overlay || base.starts_with(&overlay) || overlay.starts_with(&base) {
        return Err(RepackError::OverlappingInputs { base, overlay });
    }

    let output = resolve_output_path(&request.output_path)?;
    let sidecar = resolve_output_path(&request.manifest_path)?;
    if same_output_path(&output, &sidecar) {
        return Err(RepackError::DuplicateOutputPaths);
    }
    if same_output_path(&output, &config_path) {
        return Err(RepackError::OutputOverlapsConfig {
            path: output.clone(),
        });
    }
    if same_output_path(&sidecar, &config_path) {
        return Err(RepackError::OutputOverlapsConfig {
            path: sidecar.clone(),
        });
    }
    reject_output_inside_inputs(&output, &base, &overlay)?;
    reject_output_inside_inputs(&sidecar, &base, &overlay)?;
    create_output_parent(&output)?;
    create_output_parent(&sidecar)?;
    ensure_output_available(&output, request.force)?;
    ensure_output_available(&sidecar, request.force)?;

    let mut source_files = BTreeMap::new();
    collect_layer(&base, Layer::Base, &mut source_files)?;
    collect_layer(&overlay, Layer::Overlay, &mut source_files)?;
    reject_file_directory_collisions(&source_files)?;

    let executables: BTreeSet<String> = config
        .executable_paths
        .iter()
        .map(|path| validate_relative_path(path))
        .collect::<Result<_, _>>()?;
    for path in &executables {
        if !source_files.contains_key(path) {
            return Err(RepackError::MissingExecutable(path.clone()));
        }
    }

    let output_parent = output.parent().ok_or_else(|| RepackError::UnsafePath {
        path: output.display().to_string(),
        reason: "output must have a parent directory",
    })?;
    let staging = TempBuilder::new()
        .prefix(".stm32pkg-stage-")
        .tempdir_in(output_parent)
        .map_err(|source| io_error("create staging directory in", output_parent, source))?;
    let prepared = stage_payload(&source_files, &executables, &staging)?;
    let manifest = build_manifest(&config, &prepared)?;
    let mut manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
    manifest_bytes.push(b'\n');

    let mut archive_temp = NamedTempFile::new_in(output_parent)
        .map_err(|source| io_error("create temporary archive in", output_parent, source))?;
    write_archive(archive_temp.as_file_mut(), &prepared, &manifest_bytes)?;
    archive_temp
        .as_file_mut()
        .sync_all()
        .map_err(|source| io_error("synchronize temporary archive", archive_temp.path(), source))?;
    let archive_sha256 = sha256_file(archive_temp.path())?;

    let sidecar_parent = sidecar.parent().ok_or_else(|| RepackError::UnsafePath {
        path: sidecar.display().to_string(),
        reason: "manifest output must have a parent directory",
    })?;
    let mut sidecar_temp = NamedTempFile::new_in(sidecar_parent)
        .map_err(|source| io_error("create temporary manifest in", sidecar_parent, source))?;
    sidecar_temp
        .write_all(&manifest_bytes)
        .map_err(|source| io_error("write temporary manifest", sidecar_temp.path(), source))?;
    sidecar_temp.as_file_mut().sync_all().map_err(|source| {
        io_error(
            "synchronize temporary manifest",
            sidecar_temp.path(),
            source,
        )
    })?;

    if request.force {
        remove_existing_file(&sidecar)?;
        remove_existing_file(&output)?;
    }
    persist_output(archive_temp, &output, request.force)?;
    persist_output(sidecar_temp, &sidecar, request.force)?;

    Ok(RepackResult {
        archive: output,
        manifest: sidecar,
        archive_sha256,
        payload_sha256: manifest.payload.sha256,
        file_count: manifest.payload.file_count,
        total_size: manifest.payload.total_size,
    })
}

pub fn default_sidecar_path(output: &Path) -> PathBuf {
    let mut value = output.as_os_str().to_os_string();
    value.push(".manifest.json");
    PathBuf::from(value)
}

fn validate_identifier(field: &str, value: &str) -> Result<(), RepackError> {
    let valid = !value.is_empty()
        && value.len() <= 128
        && value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || ".-_".contains(character));
    if !valid {
        return Err(RepackError::InvalidConfig(format!(
            "{field} must be 1-128 ASCII letters, digits, dots, dashes, or underscores and start with a letter or digit"
        )));
    }
    Ok(())
}

fn validate_metadata_value(field: &str, value: &str) -> Result<(), RepackError> {
    if value.trim().is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        return Err(RepackError::InvalidConfig(format!(
            "{field} must be 1-256 visible characters"
        )));
    }
    Ok(())
}

fn validate_relative_path(value: &str) -> Result<String, RepackError> {
    if value.is_empty() {
        return Err(unsafe_path(value, "path must not be empty"));
    }
    if value.starts_with('/')
        || value.starts_with('\\')
        || value.as_bytes().get(1).is_some_and(|byte| *byte == b':')
    {
        return Err(unsafe_path(value, "absolute paths are not allowed"));
    }
    if value.contains('\\') {
        return Err(unsafe_path(
            value,
            "use forward slashes for portable relative paths",
        ));
    }

    let mut segments = Vec::new();
    for segment in value.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(unsafe_path(
                value,
                "empty, current-directory, and parent-directory segments are not allowed",
            ));
        }
        validate_archive_segment(value, segment)?;
        segments.push(segment);
    }
    let normalized = segments.join("/");
    if normalized.len() > 100 {
        return Err(unsafe_path(
            value,
            "UTF-8 path must not exceed 100 bytes for the canonical tar header",
        ));
    }
    Ok(normalized)
}

fn validate_archive_segment(full_path: &str, segment: &str) -> Result<(), RepackError> {
    if segment.contains(':') || segment.chars().any(char::is_control) {
        return Err(unsafe_path(
            full_path,
            "path segments must not contain colons or control characters",
        ));
    }
    Ok(())
}

fn unsafe_path(path: &str, reason: &'static str) -> RepackError {
    RepackError::UnsafePath {
        path: path.to_string(),
        reason,
    }
}

fn canonical_input_directory(path: &Path) -> Result<PathBuf, RepackError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|source| io_error("inspect", path, source))?;
    if metadata.file_type().is_symlink() {
        return Err(RepackError::Symlink {
            path: path.display().to_string(),
        });
    }
    if !metadata.is_dir() {
        return Err(RepackError::InputNotDirectory {
            path: path.to_path_buf(),
        });
    }
    fs::canonicalize(path).map_err(|source| io_error("canonicalize", path, source))
}

fn canonical_input_file(path: &Path) -> Result<PathBuf, RepackError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|source| io_error("inspect", path, source))?;
    if metadata.file_type().is_symlink() {
        return Err(RepackError::Symlink {
            path: path.display().to_string(),
        });
    }
    if !metadata.is_file() {
        return Err(RepackError::InputNotFile {
            path: path.to_path_buf(),
        });
    }
    fs::canonicalize(path).map_err(|source| io_error("canonicalize", path, source))
}

fn resolve_output_path(path: &Path) -> Result<PathBuf, RepackError> {
    let file_name = path
        .file_name()
        .ok_or_else(|| RepackError::UnsafePath {
            path: path.display().to_string(),
            reason: "output must name a file",
        })?
        .to_os_string();

    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|source| io_error("read current directory for", path, source))?
            .join(path)
    };
    let normalized = normalize_path(&absolute)?;
    let mut existing = normalized.parent().ok_or_else(|| RepackError::UnsafePath {
        path: path.display().to_string(),
        reason: "output must have a parent directory",
    })?;
    let mut missing = Vec::new();
    while !existing.exists() {
        let name = existing
            .file_name()
            .ok_or_else(|| RepackError::UnsafePath {
                path: path.display().to_string(),
                reason: "output has no existing ancestor",
            })?;
        missing.push(name.to_os_string());
        existing = existing.parent().ok_or_else(|| RepackError::UnsafePath {
            path: path.display().to_string(),
            reason: "output has no existing ancestor",
        })?;
    }

    let mut resolved =
        fs::canonicalize(existing).map_err(|source| io_error("canonicalize", existing, source))?;
    for component in missing.iter().rev() {
        resolved.push(component);
    }
    resolved.push(file_name);
    Ok(resolved)
}

fn normalize_path(path: &Path) -> Result<PathBuf, RepackError> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                normalized.push(component.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    return Err(RepackError::UnsafePath {
                        path: path.display().to_string(),
                        reason: "output path escapes its filesystem root",
                    });
                }
            }
        }
    }
    Ok(normalized)
}

fn same_output_path(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    #[cfg(windows)]
    {
        left.as_os_str()
            .to_string_lossy()
            .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy())
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn create_output_parent(path: &Path) -> Result<(), RepackError> {
    let parent = path.parent().ok_or_else(|| RepackError::UnsafePath {
        path: path.display().to_string(),
        reason: "output must have a parent directory",
    })?;
    fs::create_dir_all(parent).map_err(|source| io_error("create", parent, source))
}

fn reject_output_inside_inputs(
    output: &Path,
    base: &Path,
    overlay: &Path,
) -> Result<(), RepackError> {
    if output.starts_with(base) || output.starts_with(overlay) {
        return Err(RepackError::OutputInsideInput {
            path: output.to_path_buf(),
        });
    }
    Ok(())
}

fn ensure_output_available(path: &Path, force: bool) -> Result<(), RepackError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(RepackError::Symlink {
                    path: path.display().to_string(),
                });
            }
            if !force || !metadata.is_file() {
                return Err(RepackError::OutputExists {
                    path: path.to_path_buf(),
                });
            }
        }
        Err(source) if source.kind() == io::ErrorKind::NotFound => {}
        Err(source) => return Err(io_error("inspect", path, source)),
    }
    Ok(())
}

fn remove_existing_file(path: &Path) -> Result<(), RepackError> {
    if path.exists() {
        fs::remove_file(path).map_err(|source| io_error("replace", path, source))?;
    }
    Ok(())
}

fn persist_output(
    temporary: NamedTempFile,
    destination: &Path,
    allow_replace: bool,
) -> Result<(), RepackError> {
    let result = if allow_replace {
        temporary.persist(destination)
    } else {
        temporary.persist_noclobber(destination)
    };
    result.map(|_| ()).map_err(|error| RepackError::Persist {
        path: destination.to_path_buf(),
        source: error.error,
    })
}

fn collect_layer(
    root: &Path,
    layer: Layer,
    files: &mut BTreeMap<String, SourceFile>,
) -> Result<(), RepackError> {
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|error| {
            let path = error
                .path()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| root.to_path_buf());
            let source = error
                .into_io_error()
                .unwrap_or_else(|| io::Error::other("directory traversal failed"));
            io_error("walk", &path, source)
        })?;
        if entry.depth() == 0 {
            continue;
        }

        let relative = entry
            .path()
            .strip_prefix(root)
            .expect("walked entries remain under the requested root");
        let archive_path = path_to_archive(relative)?;
        if is_reserved_manifest_path(&archive_path) {
            return Err(RepackError::ReservedManifestPath(archive_path));
        }
        let file_type = entry.file_type();
        if file_type.is_symlink() {
            return Err(RepackError::Symlink { path: archive_path });
        }
        if file_type.is_dir() {
            continue;
        }
        if !file_type.is_file() {
            return Err(RepackError::UnsupportedFileType { path: archive_path });
        }

        files.insert(
            archive_path,
            SourceFile {
                source_path: entry.path().to_path_buf(),
                layer,
            },
        );
    }
    Ok(())
}

fn is_reserved_manifest_path(path: &str) -> bool {
    path.split('/')
        .next()
        .is_some_and(|segment| segment.eq_ignore_ascii_case(EMBEDDED_MANIFEST_PATH))
}

fn path_to_archive(path: &Path) -> Result<String, RepackError> {
    let mut segments = Vec::new();
    for component in path.components() {
        let Component::Normal(segment) = component else {
            return Err(unsafe_path(
                &path.display().to_string(),
                "only normal relative path segments are allowed",
            ));
        };
        let segment = segment.to_str().ok_or_else(|| {
            unsafe_path(
                &path.display().to_string(),
                "archive paths must be valid UTF-8",
            )
        })?;
        validate_archive_segment(&path.display().to_string(), segment)?;
        if segment.contains('/') || segment.contains('\\') || segment == "." || segment == ".." {
            return Err(unsafe_path(
                &path.display().to_string(),
                "path segment is not portable",
            ));
        }
        segments.push(segment);
    }
    validate_relative_path(&segments.join("/"))
}

fn reject_file_directory_collisions(
    files: &BTreeMap<String, SourceFile>,
) -> Result<(), RepackError> {
    for path in files.keys() {
        let segments: Vec<_> = path.split('/').collect();
        for length in 1..segments.len() {
            let ancestor = segments[..length].join("/");
            if files.contains_key(&ancestor) {
                return Err(RepackError::PathCollision {
                    file: ancestor,
                    descendant: path.clone(),
                });
            }
        }
    }
    Ok(())
}

fn stage_payload(
    files: &BTreeMap<String, SourceFile>,
    executables: &BTreeSet<String>,
    staging: &TempDir,
) -> Result<Vec<PreparedFile>, RepackError> {
    files
        .iter()
        .enumerate()
        .map(|(index, (archive_path, source))| {
            let staged_path = staging.path().join(format!("{index:08}.payload"));
            let (size, sha256) = copy_and_hash(&source.source_path, &staged_path)?;
            let archive_mode = if executables.contains(archive_path) {
                0o755
            } else {
                0o644
            };
            Ok(PreparedFile {
                staged_path,
                record: FileRecord {
                    path: archive_path.clone(),
                    layer: source.layer,
                    size,
                    sha256,
                    mode: format!("{archive_mode:04o}"),
                },
                archive_mode,
            })
        })
        .collect()
}

fn copy_and_hash(source: &Path, destination: &Path) -> Result<(u64, String), RepackError> {
    let mut input = File::open(source).map_err(|error| io_error("open", source, error))?;
    let mut output = File::create(destination)
        .map_err(|error| io_error("create staged copy", destination, error))?;
    let mut hasher = Sha256::new();
    let mut total_size = 0_u64;
    let mut buffer = vec![0_u8; COPY_BUFFER_SIZE];

    loop {
        let count = input
            .read(&mut buffer)
            .map_err(|error| io_error("read", source, error))?;
        if count == 0 {
            break;
        }
        output
            .write_all(&buffer[..count])
            .map_err(|error| io_error("write staged copy", destination, error))?;
        hasher.update(&buffer[..count]);
        total_size = total_size
            .checked_add(count as u64)
            .ok_or_else(|| RepackError::EntryTooLarge(source.display().to_string()))?;
    }
    output
        .sync_all()
        .map_err(|error| io_error("synchronize staged copy", destination, error))?;
    Ok((total_size, format!("{:x}", hasher.finalize())))
}

fn build_manifest(
    config: &RepackConfig,
    prepared: &[PreparedFile],
) -> Result<RepackManifest, RepackError> {
    let files: Vec<_> = prepared.iter().map(|file| file.record.clone()).collect();
    let total_size = files.iter().try_fold(0_u64, |total, file| {
        total
            .checked_add(file.size)
            .ok_or_else(|| RepackError::EntryTooLarge(file.path.clone()))
    })?;
    let payload_sha256 = payload_fingerprint(&files);

    Ok(RepackManifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        archive_format: "tar.gz".to_string(),
        generated_by: format!("stm32pkg {}", env!("CARGO_PKG_VERSION")),
        product: config.product.clone(),
        base_distribution: config.base_distribution.clone(),
        target: config.target.clone(),
        configuration_sha256: config.canonical_sha256()?,
        labels: config.labels.clone(),
        payload: PayloadSummary {
            file_count: files.len(),
            total_size,
            sha256: payload_sha256,
        },
        files,
    })
}

fn payload_fingerprint(files: &[FileRecord]) -> String {
    let mut hasher = Sha256::new();
    for file in files {
        for field in [
            file.path.as_str(),
            file.layer.as_str(),
            &file.size.to_string(),
            file.sha256.as_str(),
            file.mode.as_str(),
        ] {
            hasher.update(field.as_bytes());
            hasher.update([0]);
        }
        hasher.update(b"\n");
    }
    format!("{:x}", hasher.finalize())
}

fn write_archive(
    output: &mut File,
    prepared: &[PreparedFile],
    manifest_bytes: &[u8],
) -> Result<(), RepackError> {
    let encoder = GzBuilder::new()
        .mtime(0)
        .operating_system(255)
        .write(output, Compression::best());
    let mut archive = TarBuilder::new(encoder);

    let mut manifest_written = false;
    for file in prepared {
        if !manifest_written && EMBEDDED_MANIFEST_PATH < file.record.path.as_str() {
            append_bytes(&mut archive, EMBEDDED_MANIFEST_PATH, manifest_bytes, 0o644)?;
            manifest_written = true;
        }
        let mut input = File::open(&file.staged_path)
            .map_err(|source| io_error("open staged payload", &file.staged_path, source))?;
        append_reader(
            &mut archive,
            &file.record.path,
            &mut input,
            file.record.size,
            file.archive_mode,
        )?;
    }
    if !manifest_written {
        append_bytes(&mut archive, EMBEDDED_MANIFEST_PATH, manifest_bytes, 0o644)?;
    }

    archive
        .finish()
        .map_err(|source| io_error("finish tar archive", Path::new("<temporary>"), source))?;
    let encoder = archive
        .into_inner()
        .map_err(|source| io_error("finalize tar stream", Path::new("<temporary>"), source))?;
    encoder
        .finish()
        .map_err(|source| io_error("finalize gzip stream", Path::new("<temporary>"), source))?;
    Ok(())
}

fn append_bytes<W: Write>(
    archive: &mut TarBuilder<W>,
    path: &str,
    bytes: &[u8],
    mode: u32,
) -> Result<(), RepackError> {
    let size =
        u64::try_from(bytes.len()).map_err(|_| RepackError::EntryTooLarge(path.to_string()))?;
    append_reader(archive, path, &mut io::Cursor::new(bytes), size, mode)
}

fn append_reader<W: Write, R: Read>(
    archive: &mut TarBuilder<W>,
    path: &str,
    reader: &mut R,
    size: u64,
    mode: u32,
) -> Result<(), RepackError> {
    let mut header = Header::new_gnu();
    header
        .set_path(path)
        .map_err(|source| io_error("set archive path", Path::new(path), source))?;
    header.set_size(size);
    header.set_mode(mode);
    header.set_uid(0);
    header.set_gid(0);
    header.set_mtime(0);
    header.set_cksum();
    archive
        .append(&header, reader)
        .map_err(|source| io_error("append archive entry", Path::new(path), source))
}

fn sha256_file(path: &Path) -> Result<String, RepackError> {
    let mut file = File::open(path).map_err(|source| io_error("open", path, source))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; COPY_BUFFER_SIZE];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|source| io_error("read", path, source))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn io_error(operation: &'static str, path: &Path, source: io::Error) -> RepackError {
    RepackError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use flate2::read::GzDecoder;
    use tempfile::TempDir;

    use super::*;

    struct Fixture {
        root: TempDir,
        base: PathBuf,
        overlay: PathBuf,
        config: PathBuf,
        output: PathBuf,
        sidecar: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let root = TempDir::new().unwrap();
            let base = root.path().join("base");
            let overlay = root.path().join("overlay");
            fs::create_dir_all(base.join("config")).unwrap();
            fs::create_dir_all(overlay.join("bin")).unwrap();
            fs::create_dir_all(overlay.join("config")).unwrap();
            fs::write(base.join("base.txt"), b"base payload\n").unwrap();
            fs::write(base.join("config/settings.ini"), b"mode=base\n").unwrap();
            fs::write(overlay.join("config/settings.ini"), b"mode=product\n").unwrap();
            fs::write(overlay.join("bin/start.sh"), b"#!/bin/sh\necho start\n").unwrap();
            let config = root.path().join("product.yml");
            fs::write(
                &config,
                r#"schema_version: 1
product:
  name: traction-controller
  release: "2.4.0"
base_distribution:
  name: vendor-sdk
  release: "12.3.1"
target: stm32f407
executable_paths:
  - bin/start.sh
labels:
  profile: production
"#,
            )
            .unwrap();
            let output = root.path().join("out/product.tar.gz");
            let sidecar = root.path().join("out/product.manifest.json");
            Self {
                root,
                base,
                overlay,
                config,
                output,
                sidecar,
            }
        }

        fn request(&self) -> RepackRequest {
            RepackRequest {
                base_dir: self.base.clone(),
                overlay_dir: self.overlay.clone(),
                config_path: self.config.clone(),
                output_path: self.output.clone(),
                manifest_path: self.sidecar.clone(),
                force: false,
            }
        }
    }

    fn archive_entries(path: &Path) -> BTreeMap<String, (Vec<u8>, u32)> {
        let input = File::open(path).unwrap();
        let mut archive = tar::Archive::new(GzDecoder::new(input));
        archive
            .entries()
            .unwrap()
            .map(|entry| {
                let mut entry = entry.unwrap();
                let path = entry.path().unwrap().to_string_lossy().replace('\\', "/");
                let mode = entry.header().mode().unwrap();
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).unwrap();
                (path, (bytes, mode))
            })
            .collect()
    }

    #[test]
    fn rejects_absolute_relative_path() {
        assert!(matches!(
            validate_relative_path("/etc/passwd"),
            Err(RepackError::UnsafePath { .. })
        ));
        assert!(matches!(
            validate_relative_path("C:/Windows/system.ini"),
            Err(RepackError::UnsafePath { .. })
        ));
    }

    #[test]
    fn rejects_parent_traversal_and_backslashes() {
        for path in ["../secret", "config/../../secret", "..\\secret"] {
            assert!(matches!(
                validate_relative_path(path),
                Err(RepackError::UnsafePath { .. })
            ));
        }
    }

    #[test]
    fn accepts_portable_relative_path() {
        assert_eq!(
            validate_relative_path("config/product/settings.toml").unwrap(),
            "config/product/settings.toml"
        );
    }

    #[test]
    fn rejects_paths_that_exceed_the_canonical_tar_name_field() {
        let path = format!("{}.txt", "a".repeat(97));
        assert_eq!(path.len(), 101);
        assert!(matches!(
            validate_relative_path(&path),
            Err(RepackError::UnsafePath { .. })
        ));
    }

    #[test]
    fn rejects_invalid_configuration_schema() {
        let fixture = Fixture::new();
        let mut config = RepackConfig::from_path(&fixture.config).unwrap();
        config.schema_version = 99;
        assert!(matches!(
            config.validate(),
            Err(RepackError::InvalidConfig(_))
        ));
    }

    #[test]
    fn requires_explicit_schema_version_and_rejects_unknown_fields() {
        let fixture = Fixture::new();
        let original = fs::read_to_string(&fixture.config).unwrap();

        fs::write(
            &fixture.config,
            original.replacen("schema_version: 1\n", "", 1),
        )
        .unwrap();
        assert!(matches!(
            RepackConfig::from_path(&fixture.config),
            Err(RepackError::InvalidYaml(_))
        ));

        fs::write(
            &fixture.config,
            original.replacen(
                "schema_version: 1\n",
                "schema_version: 1\nexecutable_path: bin/start.sh\n",
                1,
            ),
        )
        .unwrap();
        assert!(matches!(
            RepackConfig::from_path(&fixture.config),
            Err(RepackError::InvalidYaml(_))
        ));
    }

    #[test]
    fn configuration_digest_normalizes_executable_order() {
        let fixture = Fixture::new();
        let mut first = RepackConfig::from_path(&fixture.config).unwrap();
        first
            .executable_paths
            .push("config/settings.ini".to_string());
        let mut second = first.clone();
        second.executable_paths.reverse();
        assert_eq!(
            first.canonical_sha256().unwrap(),
            second.canonical_sha256().unwrap()
        );
    }

    #[test]
    fn rejects_duplicate_executable_paths() {
        let fixture = Fixture::new();
        let mut config = RepackConfig::from_path(&fixture.config).unwrap();
        config.executable_paths.push("bin/start.sh".to_string());
        assert!(matches!(
            config.validate(),
            Err(RepackError::InvalidConfig(_))
        ));
    }

    #[test]
    fn rejects_unsupported_configuration_extension() {
        let fixture = Fixture::new();
        let path = fixture.root.path().join("product.toml");
        fs::write(&path, "not = 'supported'").unwrap();
        assert!(matches!(
            RepackConfig::from_path(&path),
            Err(RepackError::UnsupportedConfigExtension(_))
        ));
    }

    #[test]
    fn overlay_replaces_base_and_records_provenance() {
        let fixture = Fixture::new();
        repack_distribution(&fixture.request()).unwrap();
        let entries = archive_entries(&fixture.output);
        assert_eq!(entries["config/settings.ini"].0, b"mode=product\n");

        let manifest: RepackManifest =
            serde_json::from_slice(&entries[EMBEDDED_MANIFEST_PATH].0).unwrap();
        let settings = manifest
            .files
            .iter()
            .find(|file| file.path == "config/settings.ini")
            .unwrap();
        assert_eq!(settings.layer, Layer::Overlay);
    }

    #[test]
    fn manifest_file_records_are_sorted() {
        let fixture = Fixture::new();
        repack_distribution(&fixture.request()).unwrap();
        let manifest: RepackManifest =
            serde_json::from_slice(&fs::read(&fixture.sidecar).unwrap()).unwrap();
        let paths: Vec<_> = manifest.files.iter().map(|file| &file.path).collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted);
    }

    #[test]
    fn embedded_and_sidecar_manifests_are_identical() {
        let fixture = Fixture::new();
        repack_distribution(&fixture.request()).unwrap();
        let entries = archive_entries(&fixture.output);
        assert_eq!(
            entries[EMBEDDED_MANIFEST_PATH].0,
            fs::read(&fixture.sidecar).unwrap()
        );
    }

    #[test]
    fn archive_is_byte_for_byte_reproducible() {
        let fixture = Fixture::new();
        let first = repack_distribution(&fixture.request()).unwrap();
        let first_bytes = fs::read(&first.archive).unwrap();

        let mut second_request = fixture.request();
        second_request.output_path = fixture.root.path().join("out/second.tar.gz");
        second_request.manifest_path = fixture.root.path().join("out/second.manifest.json");
        let second = repack_distribution(&second_request).unwrap();
        assert_eq!(first_bytes, fs::read(second.archive).unwrap());
        assert_eq!(first.archive_sha256, second.archive_sha256);
    }

    #[test]
    fn configured_executable_receives_deterministic_mode() {
        let fixture = Fixture::new();
        repack_distribution(&fixture.request()).unwrap();
        let entries = archive_entries(&fixture.output);
        assert_eq!(entries["bin/start.sh"].1, 0o755);
        assert_eq!(entries["base.txt"].1, 0o644);
    }

    #[test]
    fn missing_executable_is_rejected() {
        let fixture = Fixture::new();
        let content = fs::read_to_string(&fixture.config)
            .unwrap()
            .replace("bin/start.sh", "bin/missing.sh");
        fs::write(&fixture.config, content).unwrap();
        assert!(matches!(
            repack_distribution(&fixture.request()),
            Err(RepackError::MissingExecutable(_))
        ));
    }

    #[test]
    fn existing_outputs_require_force() {
        let fixture = Fixture::new();
        repack_distribution(&fixture.request()).unwrap();
        assert!(matches!(
            repack_distribution(&fixture.request()),
            Err(RepackError::OutputExists { .. })
        ));
    }

    #[test]
    fn no_clobber_persist_preserves_a_concurrently_created_output() {
        let root = TempDir::new().unwrap();
        let destination = root.path().join("release.tar.gz");
        fs::write(&destination, b"existing\n").unwrap();
        let mut temporary = NamedTempFile::new_in(root.path()).unwrap();
        temporary.write_all(b"replacement\n").unwrap();

        assert!(matches!(
            persist_output(temporary, &destination, false),
            Err(RepackError::Persist { .. })
        ));
        assert_eq!(fs::read(&destination).unwrap(), b"existing\n");
    }

    #[test]
    fn force_replaces_existing_outputs() {
        let fixture = Fixture::new();
        repack_distribution(&fixture.request()).unwrap();
        fs::write(fixture.overlay.join("new.txt"), b"added later\n").unwrap();
        let mut request = fixture.request();
        request.force = true;
        repack_distribution(&request).unwrap();
        assert!(archive_entries(&fixture.output).contains_key("new.txt"));
    }

    #[test]
    fn output_inside_input_tree_is_rejected() {
        let fixture = Fixture::new();
        let mut request = fixture.request();
        request.output_path = fixture.base.join("generated.tar.gz");
        assert!(matches!(
            repack_distribution(&request),
            Err(RepackError::OutputInsideInput { .. })
        ));
    }

    #[test]
    fn outputs_cannot_replace_the_configuration_file() {
        let fixture = Fixture::new();
        let original = fs::read(&fixture.config).unwrap();

        let mut archive_request = fixture.request();
        archive_request.output_path = fixture.config.clone();
        archive_request.force = true;
        assert!(matches!(
            repack_distribution(&archive_request),
            Err(RepackError::OutputOverlapsConfig { .. })
        ));
        assert_eq!(fs::read(&fixture.config).unwrap(), original);

        let mut sidecar_request = fixture.request();
        sidecar_request.manifest_path = fixture.config.clone();
        sidecar_request.force = true;
        assert!(matches!(
            repack_distribution(&sidecar_request),
            Err(RepackError::OutputOverlapsConfig { .. })
        ));
        assert_eq!(fs::read(&fixture.config).unwrap(), original);
    }

    #[test]
    fn rejected_nested_output_does_not_modify_input_tree() {
        let fixture = Fixture::new();
        let nested = fixture.base.join("generated/release/product.tar.gz");
        let mut request = fixture.request();
        request.output_path = nested;
        assert!(matches!(
            repack_distribution(&request),
            Err(RepackError::OutputInsideInput { .. })
        ));
        assert!(!fixture.base.join("generated").exists());
    }

    #[test]
    fn embedded_manifest_path_and_descendants_are_reserved() {
        let fixture = Fixture::new();
        fs::write(
            fixture.base.join(EMBEDDED_MANIFEST_PATH),
            b"payload collision\n",
        )
        .unwrap();
        assert!(matches!(
            repack_distribution(&fixture.request()),
            Err(RepackError::ReservedManifestPath(_))
        ));

        let second = Fixture::new();
        let reserved_directory = second.overlay.join("package-manifest.JSON");
        fs::create_dir_all(&reserved_directory).unwrap();
        fs::write(reserved_directory.join("child.txt"), b"descendant\n").unwrap();
        assert!(matches!(
            repack_distribution(&second.request()),
            Err(RepackError::ReservedManifestPath(_))
        ));
    }

    #[test]
    fn file_directory_collision_is_rejected() {
        let fixture = Fixture::new();
        fs::write(fixture.base.join("feature"), b"base file").unwrap();
        fs::create_dir_all(fixture.overlay.join("feature")).unwrap();
        fs::write(fixture.overlay.join("feature/child.txt"), b"child").unwrap();
        assert!(matches!(
            repack_distribution(&fixture.request()),
            Err(RepackError::PathCollision { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_in_input_tree_is_rejected() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new();
        symlink(
            fixture.base.join("base.txt"),
            fixture.overlay.join("link.txt"),
        )
        .unwrap();
        assert!(matches!(
            repack_distribution(&fixture.request()),
            Err(RepackError::Symlink { .. })
        ));
    }

    #[test]
    fn default_sidecar_suffix_is_unambiguous() {
        assert_eq!(
            default_sidecar_path(Path::new("release.tar.gz")),
            PathBuf::from("release.tar.gz.manifest.json")
        );
    }
}
