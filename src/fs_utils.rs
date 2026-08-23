use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static UNIQUE_PATH_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) fn modified_unix_seconds(metadata: &fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

pub(crate) fn unique_path(parent: &Path, prefix: &str, extension: Option<&str>) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let counter = UNIQUE_PATH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let stem = format!("{prefix}-{}-{nanos}-{counter}", std::process::id());
    match extension.filter(|extension| !extension.is_empty()) {
        Some(extension) => parent.join(format!("{stem}.{extension}")),
        None => parent.join(stem),
    }
}

#[cfg(test)]
pub(crate) fn unique_test_path(label: &str) -> PathBuf {
    unique_path(&std::env::temp_dir(), &format!("lowcat-test-{label}"), None)
}
