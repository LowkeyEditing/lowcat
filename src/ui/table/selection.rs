use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::model::{FileRecord, unique_paths};

pub(crate) fn selected_records<'a>(
    records: &'a [FileRecord],
    selected: &BTreeSet<PathBuf>,
) -> Vec<&'a FileRecord> {
    records
        .iter()
        .filter(|record| selected.contains(record.path.as_path()))
        .collect()
}

pub(crate) fn records_for_path<'a>(
    records: &'a [FileRecord],
    selected: &BTreeSet<PathBuf>,
    path: &Path,
) -> Vec<&'a FileRecord> {
    if selected.len() > 1 && selected.contains(path) {
        let records = selected_records(records, selected);
        if !records.is_empty() {
            return records;
        }
    }

    records
        .iter()
        .find(|record| record.path.as_path() == path)
        .into_iter()
        .collect()
}

pub(crate) fn primary_paths<'a>(records: impl IntoIterator<Item = &'a FileRecord>) -> Vec<PathBuf> {
    records
        .into_iter()
        .map(|record| record.path.clone())
        .collect()
}

pub(crate) fn variant_paths<'a>(records: impl IntoIterator<Item = &'a FileRecord>) -> Vec<PathBuf> {
    unique_paths(
        records
            .into_iter()
            .flat_map(|record| record.variant_paths().cloned()),
    )
}

pub(crate) fn extension_paths<'a>(
    records: impl IntoIterator<Item = &'a FileRecord>,
    extension: &str,
) -> Vec<PathBuf> {
    records
        .into_iter()
        .filter_map(|record| record.variant_for_extension(extension))
        .map(|variant| variant.path.clone())
        .collect()
}
