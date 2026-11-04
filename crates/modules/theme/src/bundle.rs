//! Filesystem helpers for loading files from a theme bundle.

use std::io::Read;
use std::path::Path;

use fastcart_kernel::AppError;
use fastcart_storefront::{ThemeEditorManifest, validate_bundle_path};
use serde_json::Value;

/// Maximum accepted size of an editor manifest, in bytes.
const MAX_EDITOR_MANIFEST_BYTES: u64 = 1024 * 1024;

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
    validate_bundle_path(bundle_path).map_err(|error| AppError::validation(error.to_string()))?;
    validate_bundle_path(manifest_file).map_err(|error| AppError::validation(error.to_string()))?;

    let relative = format!("{bundle_path}/{manifest_file}");
    let root = std::fs::canonicalize(bundle_root).map_err(|error| {
        AppError::validation(format!(
            "theme bundle root is not available for {relative}: {error}"
        ))
    })?;
    let resolved = root.join(bundle_path).join(manifest_file);
    let canonical = std::fs::canonicalize(&resolved).map_err(|error| {
        AppError::validation(format!(
            "could not read theme editor manifest {relative}: {error}"
        ))
    })?;
    if !canonical.starts_with(&root) {
        return Err(AppError::validation(format!(
            "theme editor manifest escapes the bundle root: {relative}"
        )));
    }

    // The file is capped at 1 MiB, so a direct synchronous read is acceptable
    // here; the async signature keeps this aligned with the service methods.
    let file = std::fs::File::open(&canonical).map_err(|error| {
        AppError::validation(format!(
            "could not read theme editor manifest {relative}: {error}"
        ))
    })?;
    let metadata = file.metadata().map_err(|error| {
        AppError::validation(format!(
            "could not inspect theme editor manifest {relative}: {error}"
        ))
    })?;
    if !metadata.is_file() {
        return Err(AppError::validation(format!(
            "theme editor manifest is not a file: {relative}"
        )));
    }
    if metadata.len() > MAX_EDITOR_MANIFEST_BYTES {
        return Err(too_large(&relative));
    }

    let mut bytes = Vec::new();
    file.take(MAX_EDITOR_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| {
            AppError::validation(format!(
                "could not read theme editor manifest {relative}: {error}"
            ))
        })?;
    // Re-check the actual bytes read in case the file changed after the
    // metadata call.
    if bytes.len() as u64 > MAX_EDITOR_MANIFEST_BYTES {
        return Err(too_large(&relative));
    }

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

fn too_large(relative: &str) -> AppError {
    AppError::validation(format!(
        "theme editor manifest exceeds the 1 MiB limit: {relative}"
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
