use std::sync::Arc;

use fastcart_plugin::PluginError;
use fastcart_plugin::artifact::{ArchiveLimits, ArtifactStore};
use flate2::Compression;
use flate2::write::GzEncoder;
use serde_json::{Value, json};
use tar::{Builder, EntryType, Header};

fn manifest() -> Value {
    json!({
        "schema_version": "1",
        "id": "demo",
        "name": "Demo",
        "version": "1.0.0",
        "api_version": 2,
        "runtime": { "engine": "workerd", "compatibility_date": "2026-11-01" },
        "entry": { "type": "worker", "path": "worker/index.js" }
    })
}

fn store(max_module_bytes: u64) -> (tempfile::TempDir, Arc<ArtifactStore>) {
    let temp = tempfile::tempdir().expect("tempdir");
    let store = ArtifactStore::new(temp.path(), ArchiveLimits::default(), max_module_bytes);
    (temp, Arc::new(store))
}

fn append(builder: &mut Builder<&mut GzEncoder<Vec<u8>>>, path: &str, bytes: &[u8]) {
    let mut header = Header::new_gnu();
    header.set_size(bytes.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append_data(&mut header, path, bytes)
        .expect("append");
}

fn archive(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    {
        let mut builder = Builder::new(&mut encoder);
        for (path, bytes) in files {
            append(&mut builder, path, bytes);
        }
        builder.finish().expect("finish tar");
    }
    encoder.finish().expect("finish gzip")
}

fn demo_archive() -> Vec<u8> {
    archive(&[
        ("plugin.json", manifest().to_string().as_bytes()),
        (
            "worker/index.js",
            b"export default { fetch() { return new Response('ok'); } };",
        ),
    ])
}

#[test]
fn installs_and_reads_a_bundle() {
    let (_temp, store) = store(1024 * 1024);
    let stored = store.install(&demo_archive()).expect("install");
    assert_eq!(stored.manifest.id, "demo");

    let bundle = store.read_bundle(&stored.digest).expect("bundle");
    assert_eq!(bundle.main_module, "worker/index.js");
    assert!(bundle.modules.contains_key("worker/index.js"));

    let again = store.install(&demo_archive()).expect("idempotent install");
    assert_eq!(again.digest, stored.digest);
}

#[test]
fn digest_is_independent_of_archive_order() {
    let manifest = manifest().to_string();
    let first = archive(&[
        ("plugin.json", manifest.as_bytes()),
        ("worker/index.js", b"export default {};"),
    ]);
    let second = archive(&[
        ("worker/index.js", b"export default {};"),
        ("plugin.json", manifest.as_bytes()),
    ]);
    let (_temp_a, store_a) = store(1024 * 1024);
    let (_temp_b, store_b) = store(1024 * 1024);
    let a = store_a.install(&first).unwrap();
    let b = store_b.install(&second).unwrap();
    assert_eq!(a.digest, b.digest);
}

fn archive_with_raw_path(path: &str) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    {
        let mut builder = Builder::new(&mut encoder);
        append(
            &mut builder,
            "plugin.json",
            manifest().to_string().as_bytes(),
        );
        let mut header = Header::new_gnu();
        let name = path.as_bytes();
        header.as_gnu_mut().unwrap().name[..name.len()].copy_from_slice(name);
        header.set_size(4);
        header.set_mode(0o644);
        header.set_cksum();
        builder.append(&header, &b"evil"[..]).expect("append");
        builder.finish().expect("finish");
    }
    encoder.finish().expect("finish gzip")
}

#[test]
fn rejects_traversal_and_absolute_paths() {
    let (_temp, store) = store(1024 * 1024);
    for path in ["../evil.js", "/etc/passwd"] {
        let error = store
            .install(&archive_with_raw_path(path))
            .expect_err("must reject");
        assert!(matches!(error, PluginError::UnsafePath(_)), "{error:?}");
    }
}

#[test]
fn rejects_symlinks() {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    {
        let mut builder = Builder::new(&mut encoder);
        append(
            &mut builder,
            "plugin.json",
            manifest().to_string().as_bytes(),
        );
        let mut header = Header::new_gnu();
        header.set_entry_type(EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_cksum();
        builder
            .append_data(&mut header, "worker/index.js", &[][..])
            .expect("append symlink");
        builder.finish().expect("finish");
    }
    let bytes = encoder.finish().expect("gzip");
    let (_temp, store) = store(1024 * 1024);
    let error = store.install(&bytes).expect_err("must reject");
    assert!(matches!(error, PluginError::UnsafePath(_)), "{error:?}");
}

#[test]
fn rejects_duplicate_entries() {
    let manifest = manifest().to_string();
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    {
        let mut builder = Builder::new(&mut encoder);
        append(&mut builder, "plugin.json", manifest.as_bytes());
        append(&mut builder, "worker/index.js", b"export default {};");
        append(&mut builder, "worker/index.js", b"export default {};");
        builder.finish().expect("finish");
    }
    let bytes = encoder.finish().expect("gzip");
    let (_temp, store) = store(1024 * 1024);
    let error = store.install(&bytes).expect_err("must reject");
    assert!(matches!(error, PluginError::InvalidArchive(_)), "{error:?}");
}

#[test]
fn rejects_missing_entry_module() {
    let bytes = archive(&[("plugin.json", manifest().to_string().as_bytes())]);
    let (_temp, store) = store(1024 * 1024);
    let error = store.install(&bytes).expect_err("must reject");
    assert!(
        matches!(error, PluginError::InvalidManifest(_)),
        "{error:?}"
    );
}

#[test]
fn rejects_oversized_modules() {
    let big = vec![b'a'; 4096];
    let bytes = archive(&[
        ("plugin.json", manifest().to_string().as_bytes()),
        ("worker/index.js", &big),
    ]);
    let (_temp, store) = store(1024);
    let error = store.install(&bytes).expect_err("must reject");
    assert!(
        matches!(error, PluginError::ArchiveTooLarge(_)),
        "{error:?}"
    );
}

#[test]
fn detects_tampering() {
    let (temp, store) = store(1024 * 1024);
    let stored = store.install(&demo_archive()).expect("install");
    let module = temp
        .path()
        .join("artifacts")
        .join(&stored.digest)
        .join("worker/index.js");
    let mut permissions = std::fs::metadata(&module).expect("metadata").permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(0o644);
    }
    std::fs::set_permissions(&module, permissions).expect("chmod");
    std::fs::write(&module, b"tampered").expect("tamper");
    let error = store.load(&stored.digest).expect_err("must detect");
    assert!(matches!(error, PluginError::DigestMismatch(_)), "{error:?}");
}

#[test]
fn inspect_validates_without_writing() {
    let (_temp, store) = store(1024 * 1024);
    let inspected = store.inspect(&demo_archive()).expect("inspect");
    assert!(
        store.load(&inspected.digest).is_err(),
        "inspect must not store"
    );
    let stored = store.install(&demo_archive()).expect("install");
    assert_eq!(stored.digest, inspected.digest);
}

#[test]
fn prune_removes_unreferenced_artifacts() {
    use std::collections::HashSet;
    use std::time::Duration;

    let (_temp, store) = store(1024 * 1024);
    let kept = store.install(&demo_archive()).expect("install kept");
    let other = archive(&[
        ("plugin.json", manifest().to_string().as_bytes()),
        ("worker/index.js", b"export default {};\n// other"),
    ]);
    let removed = store.install(&other).expect("install other");

    let referenced = HashSet::from([kept.digest.clone()]);
    let pruned = store.prune(&referenced, Duration::ZERO).expect("prune");
    assert_eq!(pruned, vec![removed.digest.clone()]);
    assert!(store.load(&kept.digest).is_ok());
    assert!(store.load(&removed.digest).is_err());

    let fresh = store.install(&other).expect("reinstall");
    let kept_again = store
        .prune(&referenced, Duration::from_secs(3_600))
        .expect("prune with grace");
    assert!(kept_again.is_empty(), "fresh artifacts are kept");
    assert!(store.load(&fresh.digest).is_ok());
}
