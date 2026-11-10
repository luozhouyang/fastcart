//! Safe tar.gz archive reading.

use std::collections::BTreeSet;
use std::io::{Cursor, Read};
use std::path::{Component, Path};

use flate2::read::GzDecoder;

use crate::error::PluginError;

/// Limits applied while reading a plugin archive.
#[derive(Clone, Copy, Debug)]
pub struct ArchiveLimits {
    /// Maximum number of files.
    pub max_entries: usize,
    /// Maximum size of one file.
    pub max_file_bytes: u64,
    /// Maximum size of the compressed archive.
    pub max_archive_bytes: u64,
    /// Maximum total uncompressed size.
    pub max_uncompressed_bytes: u64,
}

impl Default for ArchiveLimits {
    fn default() -> Self {
        Self {
            max_entries: 2_000,
            max_file_bytes: 5 * 1024 * 1024,
            max_archive_bytes: 20 * 1024 * 1024,
            max_uncompressed_bytes: 40 * 1024 * 1024,
        }
    }
}

/// One regular file extracted from an archive.
#[derive(Clone, Debug)]
pub struct ArchiveFile {
    /// Normalized bundle-relative path.
    pub path: String,
    /// File contents.
    pub bytes: Vec<u8>,
}

/// A validated archive.
#[derive(Clone, Debug)]
pub struct Archive {
    /// Regular files in archive order.
    pub files: Vec<ArchiveFile>,
}

impl Archive {
    /// Returns the file at a path.
    #[must_use]
    pub fn file(&self, path: &str) -> Option<&ArchiveFile> {
        self.files.iter().find(|file| file.path == path)
    }
}

/// Reads and validates a gzip-compressed tar archive.
///
/// # Errors
///
/// Rejects malformed archives, unsafe paths, non-regular entries, duplicate
/// names, and archives that exceed the configured limits.
pub fn read_archive(bytes: &[u8], limits: &ArchiveLimits) -> Result<Archive, PluginError> {
    if bytes.len() as u64 > limits.max_archive_bytes {
        return Err(PluginError::ArchiveTooLarge(format!(
            "archive is {} bytes; the limit is {} bytes",
            bytes.len(),
            limits.max_archive_bytes
        )));
    }
    let decoder = GzDecoder::new(Cursor::new(bytes));
    let mut archive = tar::Archive::new(decoder);
    let entries = archive
        .entries()
        .map_err(|error| PluginError::InvalidArchive(error.to_string()))?;

    let mut files = Vec::new();
    let mut seen = BTreeSet::new();
    let mut total = 0_u64;

    for entry in entries {
        let entry = entry.map_err(|error| PluginError::InvalidArchive(error.to_string()))?;
        let entry_type = entry.header().entry_type();
        if entry_type.is_dir() {
            continue;
        }
        if !entry_type.is_file() {
            return Err(PluginError::UnsafePath(
                "only regular files are allowed in a plugin archive".to_owned(),
            ));
        }
        let path = entry
            .path()
            .map_err(|error| PluginError::InvalidArchive(error.to_string()))?;
        let path = normalize_path(&path)?;
        if !seen.insert(path.clone()) {
            return Err(PluginError::InvalidArchive(format!(
                "duplicate archive entry: {path}"
            )));
        }
        if files.len() >= limits.max_entries {
            return Err(PluginError::ArchiveTooLarge(format!(
                "archive contains more than {} entries",
                limits.max_entries
            )));
        }
        let declared = entry.size();
        if declared > limits.max_file_bytes {
            return Err(PluginError::ArchiveTooLarge(format!(
                "{path} is {declared} bytes; the per-file limit is {} bytes",
                limits.max_file_bytes
            )));
        }
        let mut buffer = Vec::with_capacity(declared as usize);
        let mut reader = entry.take(limits.max_file_bytes + 1);
        reader
            .read_to_end(&mut buffer)
            .map_err(|error| PluginError::InvalidArchive(error.to_string()))?;
        if buffer.len() as u64 > limits.max_file_bytes {
            return Err(PluginError::ArchiveTooLarge(format!(
                "{path} exceeds the per-file limit"
            )));
        }
        total += buffer.len() as u64;
        if total > limits.max_uncompressed_bytes {
            return Err(PluginError::ArchiveTooLarge(format!(
                "archive exceeds the {} byte uncompressed limit",
                limits.max_uncompressed_bytes
            )));
        }
        files.push(ArchiveFile {
            path,
            bytes: buffer,
        });
    }

    if files.is_empty() {
        return Err(PluginError::InvalidArchive(
            "archive contains no files".to_owned(),
        ));
    }
    Ok(Archive { files })
}

/// Normalizes an archive path and rejects anything outside the bundle root.
fn normalize_path(path: &Path) -> Result<String, PluginError> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => {
                let value = value
                    .to_str()
                    .ok_or_else(|| PluginError::UnsafePath("non-UTF-8 archive path".to_owned()))?;
                if value.is_empty() || value.contains('\\') || value.contains('\0') {
                    return Err(PluginError::UnsafePath(path.display().to_string()));
                }
                parts.push(value);
            }
            Component::CurDir => {
                return Err(PluginError::UnsafePath(format!(
                    "relative current-directory component: {}",
                    path.display()
                )));
            }
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(PluginError::UnsafePath(path.display().to_string()));
            }
        }
    }
    if parts.is_empty() {
        return Err(PluginError::UnsafePath("empty archive path".to_owned()));
    }
    Ok(parts.join("/"))
}
