use std::fs;
use std::path::PathBuf;

pub(crate) fn unique_path(label: &str) -> PathBuf {
    crate::fs_utils::unique_test_path(label)
}

pub(crate) fn unique_dir(label: &str) -> PathBuf {
    let path = unique_path(label);
    fs::create_dir_all(&path).expect("test directory should be creatable");
    path
}
