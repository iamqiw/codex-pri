use std::path::Path;
use std::path::PathBuf;

pub const READ_ONLY_SCRATCH_DIR_ENV_VAR: &str = "CODEX_READ_ONLY_SCRATCH_DIR";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadOnlyScratchConfig {
    pub root: PathBuf,
    pub max_bytes: u64,
    pub max_files: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadOnlyScratchScope {
    pub caller_id: String,
    pub thread_id: String,
    pub request_id: String,
    pub exec_id: String,
}

#[derive(Debug, Clone)]
pub struct ReadOnlyScratchManager {
    config: ReadOnlyScratchConfig,
}

impl ReadOnlyScratchManager {
    pub fn new(config: ReadOnlyScratchConfig) -> Self {
        Self { config }
    }

    pub fn create(&self, scope: ReadOnlyScratchScope) -> std::io::Result<ReadOnlyScratch> {
        let dir_name = [
            sanitize_path_segment(&scope.caller_id),
            sanitize_path_segment(&scope.thread_id),
            sanitize_path_segment(&scope.request_id),
            sanitize_path_segment(&scope.exec_id),
        ]
        .join("__");
        let path = self.config.root.join(dir_name);
        std::fs::create_dir_all(&path)?;

        Ok(ReadOnlyScratch {
            path,
            max_bytes: self.config.max_bytes,
            max_files: self.config.max_files,
        })
    }
}

#[derive(Debug)]
pub struct ReadOnlyScratch {
    path: PathBuf,
    max_bytes: u64,
    max_files: u64,
}

impl ReadOnlyScratch {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn max_bytes(&self) -> u64 {
        self.max_bytes
    }

    pub fn max_files(&self) -> u64 {
        self.max_files
    }
}

impl Drop for ReadOnlyScratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn sanitize_path_segment(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => ch,
            _ => '_',
        })
        .collect::<String>();
    if sanitized.is_empty() {
        "_".to_string()
    } else {
        sanitized
    }
}

#[cfg(test)]
#[path = "read_only_scratch_tests.rs"]
mod tests;
