use std::fs;
use std::process::Command;

use tempfile::TempDir;

fn create_share_layout(root: &std::path::Path) {
    let shaders_root = root.join("local-shaders");
    let playlists_root = root.join("multi");
    fs::create_dir_all(shaders_root.join("demo")).unwrap();
    fs::create_dir_all(playlists_root.join("default")).unwrap();

    fs::write(shaders_root.join("demo/shader.toml"), "name = \"Demo\"").unwrap();
    fs::write(
        playlists_root.join("default/playlist.toml"),
        "playlist = \"demo\"",
    )
    .unwrap();
    fs::write(root.join("VERSION"), "1.0.0\n").unwrap();
}

#[test]
fn defaults_sync_cli_installs_assets() {
    let root = TempDir::new().unwrap();
    let share_dir = root.path().join("share");
    let config_dir = root.path().join("config");
    let data_dir = root.path().join("data");
    let cache_dir = root.path().join("cache");

    fs::create_dir_all(&share_dir).unwrap();
    fs::create_dir_all(&config_dir).unwrap();
    fs::create_dir_all(&data_dir).unwrap();
    fs::create_dir_all(&cache_dir).unwrap();

    create_share_layout(&share_dir);

    let status = Command::new(env!("CARGO_BIN_EXE_lambdash"))
        .env("LAMBDASH_CONFIG_DIR", &config_dir)
        .env("LAMBDASH_DATA_DIR", &data_dir)
        .env("LAMBDASH_CACHE_DIR", &cache_dir)
        .env("LAMBDASH_SHARE_DIR", &share_dir)
        .args(["defaults", "sync"])
        .status()
        .expect("failed to run lambdash defaults sync");

    assert!(status.success());

    assert!(data_dir.join("local-shaders/demo/shader.toml").exists());
    assert!(data_dir.join("multi/default/playlist.toml").exists());

    let second_status = Command::new(env!("CARGO_BIN_EXE_lambdash"))
        .env("LAMBDASH_CONFIG_DIR", &config_dir)
        .env("LAMBDASH_DATA_DIR", &data_dir)
        .env("LAMBDASH_CACHE_DIR", &cache_dir)
        .env("LAMBDASH_SHARE_DIR", &share_dir)
        .args(["defaults", "sync"])
        .status()
        .expect("failed to rerun lambdash defaults sync");

    assert!(second_status.success());
}
