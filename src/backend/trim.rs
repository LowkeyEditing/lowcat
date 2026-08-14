use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::model::{AudioFormat, Category, FileVariant, TrimRange};

use super::Backend;

#[derive(Debug, Clone)]
pub(crate) struct TrimBuildRequest {
    pub source_path: PathBuf,
    pub artifact_path: PathBuf,
    pub range: TrimRange,
    pub source_size: u64,
    pub source_modified: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrimGenerationOutcome {
    Published,
    Superseded,
}

pub(crate) type TrimGenerationGate = Arc<Mutex<u64>>;

impl Backend {
    pub(crate) fn set_trim_range(
        &self,
        variants: &[FileVariant],
        range: TrimRange,
    ) -> io::Result<Vec<TrimBuildRequest>> {
        fs::create_dir_all(&self.trims_dir)?;
        let mut requests = Vec::with_capacity(variants.len());
        for variant in variants {
            let artifact_path = self.trim_artifact_path(&variant.path)?;
            self.db.set_trim(
                &variant.path,
                range,
                &artifact_path,
                variant.size,
                variant.modified,
            )?;
            requests.push(TrimBuildRequest {
                source_path: variant.path.clone(),
                artifact_path,
                range,
                source_size: variant.size,
                source_modified: variant.modified,
            });
        }
        Ok(requests)
    }

    pub(crate) fn clear_trims(&self, paths: &[PathBuf]) -> io::Result<()> {
        for artifact in self.db.clear_trims(paths)? {
            remove_file_if_present(&artifact)?;
        }
        Ok(())
    }

    pub(crate) fn transfer_trim(&self, source: &Path, destination: &Path) -> io::Result<bool> {
        let Some(trim) = self.db.trim_for_path(source)? else {
            return Ok(false);
        };
        let metadata = fs::metadata(destination)?;
        let artifact_path = self.trim_artifact_path(destination)?;
        if trim.artifact_path != artifact_path && trim.artifact_path.is_file() {
            fs::create_dir_all(&self.trims_dir)?;
            if let Err(error) = fs::rename(&trim.artifact_path, &artifact_path) {
                let _ = fs::remove_file(&trim.artifact_path);
                eprintln!(
                    "lowcat trim artifact transfer failed source={} destination={} error={error}",
                    trim.artifact_path.display(),
                    artifact_path.display()
                );
            }
        }
        self.db.transfer_trim(
            source,
            destination,
            &artifact_path,
            metadata.len(),
            modified_secs(&metadata),
        )
    }

    pub(crate) fn copy_trim(&self, source: &Path, destination: &Path) -> io::Result<bool> {
        let metadata = fs::metadata(destination)?;
        let artifact_path = self.trim_artifact_path(destination)?;
        self.db.copy_trim(
            source,
            destination,
            &artifact_path,
            metadata.len(),
            modified_secs(&metadata),
        )
    }

    pub(crate) fn generate_trim_artifact(
        &self,
        request: &TrimBuildRequest,
        gate: &TrimGenerationGate,
        token: u64,
    ) -> io::Result<TrimGenerationOutcome> {
        let initial = source_fingerprint(&request.source_path)?;
        if initial != (request.source_size, request.source_modified) {
            return Err(io::Error::other("trim source changed before generation"));
        }
        let duration = media_duration_seconds(&request.source_path)
            .ok_or_else(|| io::Error::other("could not read trim source duration"))?;
        let start = duration * request.range.start_ratio as f64;
        let selected_duration =
            duration * (request.range.end_ratio - request.range.start_ratio) as f64;
        if !selected_duration.is_finite() || selected_duration <= 0. {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "trim range has no duration",
            ));
        }

        fs::create_dir_all(&self.trims_dir)?;
        let extension = request
            .source_path
            .extension()
            .and_then(|extension| extension.to_str())
            .and_then(|extension| extension.parse::<AudioFormat>().ok())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "unsupported trim format")
            })?;
        let temp_path = unique_temp_path(&self.trims_dir, extension.extension());
        let output_args: &[&str] = match extension {
            AudioFormat::Mp3 => &["-vn", "-c:a", "libmp3lame", "-q:a", "2", "-y"],
            AudioFormat::Wav => &["-vn", "-c:a", "pcm_s16le", "-y"],
            AudioFormat::Opus => &["-vn", "-c:a", "libopus", "-y"],
            AudioFormat::Flac => &["-vn", "-c:a", "flac", "-y"],
        };
        let status = crate::media_tools::command("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-i"])
            .arg(&request.source_path)
            .args([
                "-ss",
                &format!("{start:.9}"),
                "-t",
                &format!("{selected_duration:.9}"),
            ])
            .args(output_args)
            .arg(&temp_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        match status {
            Ok(status) if status.success() && temp_path.is_file() => {}
            Ok(_) => {
                let _ = fs::remove_file(&temp_path);
                return Err(io::Error::other("ffmpeg trim generation failed"));
            }
            Err(error) => {
                let _ = fs::remove_file(&temp_path);
                return Err(error);
            }
        }

        if source_fingerprint(&request.source_path)? != initial {
            let _ = fs::remove_file(&temp_path);
            return Err(io::Error::other("trim source changed during generation"));
        }

        let current_token = gate.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if *current_token != token {
            drop(current_token);
            let _ = fs::remove_file(&temp_path);
            return Ok(TrimGenerationOutcome::Superseded);
        }
        if let Err(error) = fs::rename(&temp_path, &request.artifact_path) {
            let _ = fs::remove_file(&temp_path);
            return Err(error);
        }
        if !self.db.mark_trim_artifact_ready(
            &request.source_path,
            request.range,
            request.source_size,
            request.source_modified,
        )? {
            return Ok(TrimGenerationOutcome::Superseded);
        }
        Ok(TrimGenerationOutcome::Published)
    }

    pub(super) fn reconcile_trim_records(&self, category: Category) -> io::Result<()> {
        fs::create_dir_all(&self.trims_dir)?;
        for candidate in self.db.trim_inheritance_candidates(category)? {
            let artifact_path = self.trim_artifact_path(&candidate.source_path)?;
            self.db.set_trim(
                &candidate.source_path,
                candidate.range,
                &artifact_path,
                candidate.size,
                candidate.modified,
            )?;
        }
        Ok(())
    }

    pub(crate) fn reconcile_orphan_trims(&self) -> io::Result<()> {
        for artifact in self.db.remove_orphan_trims()? {
            remove_file_if_present(&artifact)?;
        }
        Ok(())
    }

    fn trim_artifact_path(&self, source: &Path) -> io::Result<PathBuf> {
        let extension = source
            .extension()
            .and_then(|extension| extension.to_str())
            .and_then(|extension| extension.parse::<AudioFormat>().ok())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "unsupported trim format")
            })?;
        Ok(self.trims_dir.join(format!(
            "{:016x}.{}",
            stable_path_hash(source),
            extension.extension()
        )))
    }
}

fn media_duration_seconds(path: &Path) -> Option<f64> {
    let output = crate::media_tools::command("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=noprint_wrappers=1:nokey=1",
        ])
        .arg(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let duration = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .ok()?;
    (duration.is_finite() && duration > 0.).then_some(duration)
}

fn stable_path_hash(path: &Path) -> u64 {
    path.as_os_str()
        .as_encoded_bytes()
        .iter()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
}

fn unique_temp_path(folder: &Path, extension: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    folder.join(format!(
        ".lowcat-trim-{}-{nanos}.{extension}",
        std::process::id()
    ))
}

fn source_fingerprint(path: &Path) -> io::Result<(u64, i64)> {
    let metadata = fs::metadata(path)?;
    Ok((metadata.len(), modified_secs(&metadata)))
}

fn modified_secs(metadata: &fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|modified| modified.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

fn remove_file_if_present(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}
