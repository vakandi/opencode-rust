//! Port of upstream packages/opencode/test/config/config.test.ts behaviour.
//! Tests config loading: JSON/JSONC, merge precedence, env substitution.

use opencode_config::{ConfigLoader, Config};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

fn write_config(dir: &PathBuf, content: &str, name: &str) {
    let path = dir.join(name);
    fs::write(path, content).unwrap();
}

#[test]
fn loads_json_config_file() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().to_path_buf();
    write_config(
        &dir,
        r#"{"$schema":"https://opencode.ai/config.json","model":"test/model","username":"testuser"}"#,
        "opencode.json",
    );

    let mut loader = ConfigLoader::new();
    let config = loader.load_all(&dir).unwrap();

    assert_eq!(config.model.as_deref(), Some("test/model"));
    assert_eq!(config.username.as_deref(), Some("testuser"));
}

#[test]
fn loads_jsonc_config_file() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().to_path_buf();
    write_config(
        &dir,
        r#"{
        // This is a comment
        "$schema": "https://opencode.ai/config.json",
        "model": "test/model",
        "username": "testuser"
    }"#,
        "opencode.jsonc",
    );

    let mut loader = ConfigLoader::new();
    loader.load_from_file(dir.join("opencode.jsonc")).unwrap();
    let config = loader.load_all(&dir).unwrap();

    assert_eq!(config.model.as_deref(), Some("test/model"));
    assert_eq!(config.username.as_deref(), Some("testuser"));
}

#[test]
fn merges_multiple_config_files_with_correct_precedence() {
    // Later-loaded override earlier. Project loads .jsonc then .json; .json wins for overlapping keys.
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().to_path_buf();
    write_config(
        &dir,
        r#"{"$schema":"https://opencode.ai/config.json","model":"base","username":"base"}"#,
        "opencode.jsonc",
    );
    write_config(
        &dir,
        r#"{"$schema":"https://opencode.ai/config.json","model":"override"}"#,
        "opencode.json",
    );

    let mut loader = ConfigLoader::new();
    let config = loader.load_all(&dir).unwrap();

    assert_eq!(config.model.as_deref(), Some("override"));
    assert_eq!(config.username.as_deref(), Some("base"));
}

#[test]
fn load_from_str_parses_and_merges() {
    let mut loader = ConfigLoader::new();
    loader
        .load_from_str(r#"{"model":"anthropic/claude-sonnet-4-20250514"}"#)
        .unwrap();
    let config = loader.get_config();
    assert_eq!(
        config.model.as_deref(),
        Some("anthropic/claude-sonnet-4-20250514")
    );
}
