use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;

fn project_root() -> PathBuf {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    crate_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("crate dir has workspace root")
        .to_path_buf()
}

#[test]
fn installer_script_copies_defaults() {
    let root = project_root();
    let script_path = root.join("scripts/install.sh");

    assert!(
        script_path.exists(),
        "expected install script at {:?}",
        script_path
    );

    let prefix = TempDir::new().unwrap();
    let data_dir = TempDir::new().unwrap();

    let status = Command::new("bash")
        .current_dir(&root)
        .arg(script_path.to_str().unwrap())
        .arg("--source")
        .arg(root.to_str().unwrap())
        .arg("--skip-build")
        .arg("--prefix")
        .arg(prefix.path().to_str().unwrap())
        .arg("--data-dir")
        .arg(data_dir.path().to_str().unwrap())
        .status()
        .expect("failed to launch installer script");

    assert!(
        status.success(),
        "installer script returned non-zero status"
    );

    let data_dir = data_dir.path();
    assert!(data_dir.join("local-shaders").exists());
    assert!(data_dir
        .join("local-shaders/playlists/workspaces.toml")
        .exists());
}
