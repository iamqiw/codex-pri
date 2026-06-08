use std::fs;

use pretty_assertions::assert_eq;
use tempfile::TempDir;

use super::ReadOnlyScratchConfig;
use super::ReadOnlyScratchManager;
use super::ReadOnlyScratchScope;

#[test]
fn scratch_directory_is_scoped_under_configured_root_and_removed_on_drop() {
    let root = TempDir::new().expect("scratch root");
    let manager = ReadOnlyScratchManager::new(ReadOnlyScratchConfig {
        root: root.path().join("scratch"),
        max_bytes: 1024,
        max_files: 8,
    });

    let scratch = manager
        .create(ReadOnlyScratchScope {
            caller_id: "caller/../raw user input".to_string(),
            thread_id: "thread-1".to_string(),
            request_id: "request-1".to_string(),
            exec_id: "exec-1".to_string(),
        })
        .expect("scratch directory");
    let scratch_path = scratch.path().to_path_buf();

    assert!(scratch_path.starts_with(root.path().join("scratch")));
    assert!(scratch_path.exists());
    assert_eq!(scratch.max_bytes(), 1024);
    assert_eq!(scratch.max_files(), 8);
    let scratch_dir_name = scratch_path
        .file_name()
        .expect("scratch path should have a final component")
        .to_string_lossy();
    assert!(!scratch_dir_name.contains(".."));
    assert!(!scratch_dir_name.contains('/'));

    fs::write(scratch_path.join("tool.tmp"), b"scratch").expect("write scratch file");
    drop(scratch);

    assert!(!scratch_path.exists());
}
