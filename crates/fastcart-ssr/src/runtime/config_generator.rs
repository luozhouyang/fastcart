//! Generates a workerd Cap'n Proto configuration for a stored artifact.

use std::path::Path;

use crate::SsrError;
use crate::artifact::Artifact;
use crate::config::WorkerdConfig;

/// Generates `config.capnp` for one artifact and returns the process config.
pub fn generate_workerd_config(
    config_dir: &Path,
    artifact: &Artifact,
    listen_addr: std::net::SocketAddr,
    binary: &Path,
) -> Result<WorkerdConfig, SsrError> {
    std::fs::create_dir_all(config_dir)
        .map_err(|error| SsrError::InvalidRuntimeConfig(error.to_string()))?;
    let modules = collect_modules(artifact, config_dir)?;
    let config_path = config_dir.join("config.capnp");
    let manifest = artifact.manifest();
    let flags = manifest
        .runtime
        .compatibility_flags
        .iter()
        .map(|flag| quote(flag))
        .collect::<Vec<_>>()
        .join(", ");
    let modules_text = modules
        .iter()
        .map(|(name, path)| {
            format!(
                "    (name = {}, esModule = embed {}),",
                quote(name),
                quote(path)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    let text = format!(
        r#"using Workerd = import "/workerd/workerd.capnp";

const config :Workerd.Config = (
  services = [
    (name = "main", worker = .mainWorker),
  ],
  sockets = [
    (name = "http", address = {}, http = (), service = "main"),
  ],
);

const mainWorker :Workerd.Worker = (
  modules = [
{modules_text}
  ],
  compatibilityDate = {},
  compatibilityFlags = [{flags}],
);
"#,
        quote(&listen_addr.to_string()),
        quote(&manifest.runtime.compatibility_date),
    );
    std::fs::write(&config_path, text)
        .map_err(|error| SsrError::InvalidRuntimeConfig(error.to_string()))?;
    let mut config = WorkerdConfig::new(binary, config_path, listen_addr);
    config.health_path = "/__fastcart/health".to_owned();
    Ok(config)
}

fn collect_modules(
    artifact: &Artifact,
    config_dir: &Path,
) -> Result<Vec<(String, String)>, SsrError> {
    let root = artifact.root();
    let mut files = Vec::new();
    walk(root, root, &mut files)?;
    files.sort();
    let entry = artifact.manifest().entry.path.as_str();
    let entry_path = root.join(entry);
    let mut modules = Vec::with_capacity(files.len());
    for relative in files {
        let file = root.join(&relative);
        if !matches!(
            file.extension().and_then(std::ffi::OsStr::to_str),
            Some("js" | "mjs")
        ) {
            continue;
        }
        let embed = pathdiff::diff_paths(&file, config_dir)
            .ok_or_else(|| SsrError::InvalidRuntimeConfig("module path escapes".to_owned()))?;
        let embed = embed
            .to_str()
            .ok_or_else(|| SsrError::InvalidRuntimeConfig("non-UTF-8 module path".to_owned()))?
            .replace('\\', "/");
        modules.push((relative, embed));
    }
    let entry_relative = artifact.manifest().entry.path.clone();
    let Some(index) = modules.iter().position(|(name, _)| name == &entry_relative) else {
        return Err(SsrError::InvalidRuntimeConfig(format!(
            "entry is not a JavaScript module: {}",
            entry_path.display()
        )));
    };
    let entry_module = modules.remove(index);
    modules.insert(0, entry_module);
    Ok(modules)
}

fn walk(root: &Path, directory: &Path, files: &mut Vec<String>) -> Result<(), SsrError> {
    let entries = std::fs::read_dir(directory)
        .map_err(|error| SsrError::InvalidRuntimeConfig(error.to_string()))?;
    for entry in entries {
        let entry = entry.map_err(|error| SsrError::InvalidRuntimeConfig(error.to_string()))?;
        let path = entry.path();
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| SsrError::InvalidRuntimeConfig(error.to_string()))?;
        if metadata.file_type().is_symlink() {
            return Err(SsrError::InvalidRuntimeConfig(format!(
                "symlink in artifact: {}",
                path.display()
            )));
        }
        if metadata.is_dir() {
            walk(root, &path, files)?;
        } else if metadata.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| SsrError::InvalidRuntimeConfig("path escape".to_owned()))?
                .to_string_lossy()
                .replace('\\', "/");
            files.push(relative);
        }
    }
    Ok(())
}

fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}
