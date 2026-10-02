use std::fs::File;

use benchmark_api::features::v1::benchmark::discovery;
use tempfile::TempDir;

fn touch(dir: &TempDir, name: &str) {
    File::create(dir.path().join(name)).unwrap();
}

#[test]
fn no_recognized_manifest_returns_null_runtime() {
    let dir = TempDir::new().unwrap();

    let result = discovery::discover(dir.path()).unwrap();

    assert_eq!(result.runtime, None);
}

#[test]
fn package_json_detects_node() {
    let dir = TempDir::new().unwrap();
    touch(&dir, "package.json");

    let result = discovery::discover(dir.path()).unwrap();

    assert_eq!(result.runtime.as_deref(), Some("node"));
}

#[test]
fn bun_lock_detects_bun() {
    let dir = TempDir::new().unwrap();
    touch(&dir, "bun.lock");

    let result = discovery::discover(dir.path()).unwrap();

    assert_eq!(result.runtime.as_deref(), Some("bun"));
}

#[test]
fn bun_lockb_detects_bun() {
    let dir = TempDir::new().unwrap();
    touch(&dir, "bun.lockb");

    let result = discovery::discover(dir.path()).unwrap();

    assert_eq!(result.runtime.as_deref(), Some("bun"));
}

#[test]
fn package_json_and_bun_lock_detect_bun() {
    let dir = TempDir::new().unwrap();
    touch(&dir, "package.json");
    touch(&dir, "bun.lock");

    let result = discovery::discover(dir.path()).unwrap();

    assert_eq!(result.runtime.as_deref(), Some("bun"));
}

#[test]
fn package_json_and_cargo_toml_fail() {
    let dir = TempDir::new().unwrap();
    touch(&dir, "package.json");
    touch(&dir, "Cargo.toml");

    assert!(discovery::discover(dir.path()).is_err());
}

#[test]
fn cargo_toml_detects_rust() {
    let dir = TempDir::new().unwrap();
    touch(&dir, "Cargo.toml");

    let result = discovery::discover(dir.path()).unwrap();

    assert_eq!(result.runtime.as_deref(), Some("rust"));
}

#[test]
fn go_mod_detects_go() {
    let dir = TempDir::new().unwrap();
    touch(&dir, "go.mod");

    let result = discovery::discover(dir.path()).unwrap();

    assert_eq!(result.runtime.as_deref(), Some("go"));
}

#[test]
fn pyproject_toml_detects_python() {
    let dir = TempDir::new().unwrap();
    touch(&dir, "pyproject.toml");

    let result = discovery::discover(dir.path()).unwrap();

    assert_eq!(result.runtime.as_deref(), Some("python"));
}

#[test]
fn nested_manifests_are_ignored() {
    let dir = TempDir::new().unwrap();

    std::fs::create_dir_all(dir.path().join("frontend")).unwrap();
    std::fs::create_dir_all(dir.path().join("backend")).unwrap();

    File::create(dir.path().join("frontend/package.json")).unwrap();
    File::create(dir.path().join("backend/Cargo.toml")).unwrap();

    let result = discovery::discover(dir.path()).unwrap();

    assert_eq!(result.runtime, None);
}
