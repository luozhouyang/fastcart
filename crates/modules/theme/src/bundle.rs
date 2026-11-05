//! Filesystem helpers for loading files from a theme bundle.

use std::io::Read;
use std::path::Path;

use fastcart_kernel::AppError;
use fastcart_storefront::{ThemeEditorManifest, validate_bundle_path};
use serde_json::Value;

/// Maximum accepted size of an editor manifest, in bytes.
const MAX_EDITOR_MANIFEST_BYTES: u64 = 1024 * 1024;

/// Maximum accepted size of a general theme bundle file, in bytes.
const MAX_BUNDLE_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// Returns `true` when `bundle_path` resolves to a directory inside the
/// canonical bundle root.
///
/// Registration uses this to decide whether a missing manifest file should be
/// a validation error: deployments point `themes_root` at a real directory,
/// while tests may use bundle paths that do not exist on disk.
#[must_use]
pub fn bundle_directory_exists(bundle_root: &Path, bundle_path: &str) -> bool {
    if validate_bundle_path(bundle_path).is_err() {
        return false;
    }
    let Ok(root) = std::fs::canonicalize(bundle_root) else {
        return false;
    };
    let Ok(candidate) = std::fs::canonicalize(root.join(bundle_path)) else {
        return false;
    };
    candidate.starts_with(&root) && candidate.is_dir()
}

/// Loads and validates a theme's editor manifest from its bundle on disk.
///
/// `bundle_path` and `manifest_file` are bundle-relative and are validated by
/// [`fastcart_storefront::validate_bundle_path`]. The resolved file must stay
/// inside the canonical bundle root, must be a regular file and may not exceed
/// 1 MiB.
pub async fn load_editor_manifest(
    bundle_root: &Path,
    bundle_path: &str,
    manifest_file: &str,
) -> Result<ThemeEditorManifest, AppError> {
    let bytes = read_bundle_file(
        bundle_root,
        bundle_path,
        manifest_file,
        MAX_EDITOR_MANIFEST_BYTES,
        "theme editor manifest",
    )?;
    let relative = format!("{bundle_path}/{manifest_file}");
    let value: Value = serde_json::from_slice(&bytes).map_err(|error| {
        AppError::validation(format!(
            "invalid theme editor manifest JSON in {relative}: {error}"
        ))
    })?;
    // `parse` performs structural parsing and calls `validate` internally.
    ThemeEditorManifest::parse(&value).map_err(|error| {
        AppError::validation(format!("invalid theme editor manifest {relative}: {error}"))
    })
}

/// Reads one file from a theme bundle with canonical root and traversal checks.
///
/// The resolved file must stay inside the canonical bundle root, must be a
/// regular file and may not exceed 4 MiB. Callers choose the bundle-relative
/// path from trusted manifest data, never from raw request input.
pub async fn load_theme_bundle_file(
    bundle_root: &Path,
    bundle_path: &str,
    relative_path: &str,
) -> Result<Vec<u8>, AppError> {
    read_bundle_file(
        bundle_root,
        bundle_path,
        relative_path,
        MAX_BUNDLE_FILE_BYTES,
        "theme bundle file",
    )
}

/// Loads a theme's assets manifest as JSON.
pub async fn load_assets_manifest(
    bundle_root: &Path,
    bundle_path: &str,
    manifest_file: &str,
) -> Result<Value, AppError> {
    let bytes = read_bundle_file(
        bundle_root,
        bundle_path,
        manifest_file,
        MAX_BUNDLE_FILE_BYTES,
        "theme assets manifest",
    )?;
    let relative = format!("{bundle_path}/{manifest_file}");
    serde_json::from_slice(&bytes).map_err(|error| {
        AppError::validation(format!(
            "invalid theme assets manifest JSON in {relative}: {error}"
        ))
    })
}

/// Loads a theme's settings JSON Schema and verifies it is a JSON object.
pub async fn load_settings_schema(
    bundle_root: &Path,
    bundle_path: &str,
    schema_file: &str,
) -> Result<Value, AppError> {
    let bytes = read_bundle_file(
        bundle_root,
        bundle_path,
        schema_file,
        MAX_BUNDLE_FILE_BYTES,
        "theme settings schema",
    )?;
    let relative = format!("{bundle_path}/{schema_file}");
    let value: Value = serde_json::from_slice(&bytes).map_err(|error| {
        AppError::validation(format!(
            "invalid theme settings schema JSON in {relative}: {error}"
        ))
    })?;
    if !value.is_object() {
        return Err(AppError::validation(format!(
            "theme settings schema must be a JSON object: {relative}"
        )));
    }
    Ok(value)
}

fn read_bundle_file(
    bundle_root: &Path,
    bundle_path: &str,
    relative_path: &str,
    max_bytes: u64,
    kind: &str,
) -> Result<Vec<u8>, AppError> {
    validate_bundle_path(bundle_path).map_err(|error| AppError::validation(error.to_string()))?;
    validate_bundle_path(relative_path).map_err(|error| AppError::validation(error.to_string()))?;

    let relative = format!("{bundle_path}/{relative_path}");
    let root = std::fs::canonicalize(bundle_root).map_err(|error| {
        AppError::validation(format!(
            "{kind} root is not available for {relative}: {error}"
        ))
    })?;
    let bundle_root = std::fs::canonicalize(root.join(bundle_path)).map_err(|error| {
        AppError::validation(format!(
            "{kind} bundle is not available for {relative}: {error}"
        ))
    })?;
    if !bundle_root.starts_with(&root) {
        return Err(AppError::validation(format!(
            "{kind} bundle escapes the theme root: {relative}"
        )));
    }
    let canonical = std::fs::canonicalize(bundle_root.join(relative_path)).map_err(|error| {
        AppError::validation(format!("could not read {kind} {relative}: {error}"))
    })?;
    if !canonical.starts_with(&bundle_root) {
        return Err(AppError::validation(format!(
            "{kind} escapes the theme bundle: {relative}"
        )));
    }

    // Files are capped at a few MiB, so a direct synchronous read is acceptable
    // here; the async signatures keep this aligned with the service methods.
    let file = std::fs::File::open(&canonical).map_err(|error| {
        AppError::validation(format!("could not read {kind} {relative}: {error}"))
    })?;
    let metadata = file.metadata().map_err(|error| {
        AppError::validation(format!("could not inspect {kind} {relative}: {error}"))
    })?;
    if !metadata.is_file() {
        return Err(AppError::validation(format!(
            "{kind} is not a file: {relative}"
        )));
    }
    if metadata.len() > max_bytes {
        return Err(too_large(kind, &relative, max_bytes));
    }

    let mut bytes = Vec::new();
    file.take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            AppError::validation(format!("could not read {kind} {relative}: {error}"))
        })?;
    // Re-check the actual bytes read in case the file changed after the
    // metadata call.
    if bytes.len() as u64 > max_bytes {
        return Err(too_large(kind, &relative, max_bytes));
    }
    Ok(bytes)
}

fn too_large(kind: &str, relative: &str, max_bytes: u64) -> AppError {
    AppError::validation(format!(
        "{kind} exceeds the {} MiB limit: {relative}",
        max_bytes / (1024 * 1024)
    ))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use fastcart_kernel::AppError;
    use serde_json::{Value, json};

    use super::{MAX_EDITOR_MANIFEST_BYTES, load_editor_manifest};

    fn manifest() -> Value {
        json!({
            "schema_version": 1,
            "categories": {
                "hero": { "title": "Hero", "components": ["Hero"] }
            },
            "components": {
                "Hero": {
                    "title": "Hero",
                    "category": "hero",
                    "fields": {
                        "title": { "type": "text", "required": true }
                    }
                }
            }
        })
    }

    fn write_manifest(root: &Path, bundle_path: &str, manifest_file: &str, value: &Value) {
        let dir = root.join(bundle_path);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(manifest_file), serde_json::to_vec(value).unwrap()).unwrap();
    }

    fn assert_validation(error: &AppError) {
        assert!(matches!(error, AppError::Validation(_)), "{error:?}");
    }

    #[tokio::test]
    async fn loads_a_valid_editor_manifest() {
        let root = tempfile::tempdir().unwrap();
        write_manifest(
            root.path(),
            "themes/default",
            "editor.manifest.json",
            &manifest(),
        );

        let loaded = load_editor_manifest(root.path(), "themes/default", "editor.manifest.json")
            .await
            .unwrap();

        assert_eq!(loaded.schema_version, 1);
        assert!(loaded.components.contains_key("Hero"));
    }

    #[tokio::test]
    async fn rejects_a_missing_manifest_file() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("themes/default")).unwrap();

        let error = load_editor_manifest(root.path(), "themes/default", "editor.manifest.json")
            .await
            .unwrap_err();
        assert_validation(&error);
    }

    #[tokio::test]
    async fn rejects_invalid_manifest_json() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("themes/default");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("editor.manifest.json"), b"{ not json").unwrap();

        let error = load_editor_manifest(root.path(), "themes/default", "editor.manifest.json")
            .await
            .unwrap_err();
        assert_validation(&error);
    }

    #[tokio::test]
    async fn rejects_traversal_paths() {
        let root = tempfile::tempdir().unwrap();
        write_manifest(
            root.path(),
            "themes/default",
            "editor.manifest.json",
            &manifest(),
        );

        let error = load_editor_manifest(root.path(), "themes/default", "../outside.json")
            .await
            .unwrap_err();
        assert_validation(&error);

        let error = load_editor_manifest(root.path(), "../outside", "editor.manifest.json")
            .await
            .unwrap_err();
        assert_validation(&error);

        let error = load_editor_manifest(root.path(), "themes/default", "/etc/passwd")
            .await
            .unwrap_err();
        assert_validation(&error);
    }

    #[tokio::test]
    async fn rejects_manifests_over_one_mib() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("themes/default");
        std::fs::create_dir_all(&dir).unwrap();
        let file = std::fs::File::create(dir.join("editor.manifest.json")).unwrap();
        file.set_len(MAX_EDITOR_MANIFEST_BYTES + 1).unwrap();

        let error = load_editor_manifest(root.path(), "themes/default", "editor.manifest.json")
            .await
            .unwrap_err();
        assert_validation(&error);
    }

    #[tokio::test]
    async fn rejects_manifests_that_fail_editor_validation() {
        let root = tempfile::tempdir().unwrap();
        write_manifest(
            root.path(),
            "themes/default",
            "editor.manifest.json",
            &json!({ "schema_version": 1, "components": {} }),
        );

        let error = load_editor_manifest(root.path(), "themes/default", "editor.manifest.json")
            .await
            .unwrap_err();
        assert_validation(&error);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_symlink_escapes() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_file = outside.path().join("editor.manifest.json");
        std::fs::write(&outside_file, serde_json::to_vec(&manifest()).unwrap()).unwrap();

        let dir = root.path().join("themes/default");
        std::fs::create_dir_all(&dir).unwrap();
        std::os::unix::fs::symlink(&outside_file, dir.join("editor.manifest.json")).unwrap();

        let error = load_editor_manifest(root.path(), "themes/default", "editor.manifest.json")
            .await
            .unwrap_err();
        assert_validation(&error);
    }
}
