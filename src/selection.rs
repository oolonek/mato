use std::cmp::Ordering;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use csv::{ReaderBuilder, StringRecord, WriterBuilder};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Selection {
    #[serde(default)]
    pub(crate) require_nonempty: Vec<String>,
    #[serde(default)]
    pub(crate) sort: Vec<SortKey>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SortKey {
    column: String,
    #[serde(default)]
    descending: bool,
}

impl Selection {
    fn validate(&self) -> Result<()> {
        ensure!(!self.require_nonempty.is_empty() || !self.sort.is_empty(),
            "[selection] must specify require_nonempty or at least one [[selection.sort]]");
        for column in self.require_nonempty.iter().chain(self.sort.iter().map(|key| &key.column)) {
            ensure!(!column.trim().is_empty() && column == column.trim(),
                "selection column names must be nonempty exact metadata headers: {column:?}");
        }
        Ok(())
    }
}

fn source_hash(path: &Path) -> Result<[u8; 32]> {
    let mut source = File::open(path).with_context(|| format!("opening source metadata {}", path.display()))?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let length = source.read(&mut buffer)?;
        if length == 0 { break; }
        hash.update(&buffer[..length]);
    }
    Ok(hash.finalize().into())
}

pub(crate) fn path(dataset_root: &Path, source: &Path, selection: &Selection, limit: usize) -> Result<PathBuf> {
    selection.validate()?;
    ensure!(limit > 0, "--topN must be positive");
    let mut hash = Sha256::new();
    hash.update(b"mato-metadata-selection-v1");
    hash.update(source_hash(source)?);
    hash.update((limit as u64).to_be_bytes());
    hash.update(serde_json::to_vec(selection)?);
    let extension = if source.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("csv")) { "csv" } else { "tsv" };
    Ok(dataset_root.join(".mato-selections").join(format!("selected-{:x}.{extension}", hash.finalize())))
}

fn column(headers: &StringRecord, name: &str) -> Result<usize> {
    headers.iter().position(|header| header == name)
        .with_context(|| format!("selection column {name:?} absent from metadata header"))
}

fn identical(left: &Path, right: &Path) -> Result<bool> {
    if fs::metadata(left)?.len() != fs::metadata(right)?.len() { return Ok(false); }
    let mut left = File::open(left)?;
    let mut right = File::open(right)?;
    let mut a = [0; 64 * 1024];
    let mut b = [0; 64 * 1024];
    loop {
        let count = left.read(&mut a)?;
        if count == 0 { return Ok(true); }
        right.read_exact(&mut b[..count])?;
        if a[..count] != b[..count] { return Ok(false); }
    }
}

/// Write an immutable selected metadata snapshot beneath the source dataset root, where
/// Machaca and Peregrino accept explicitly supplied metadata without weakening their guards.
pub(crate) fn materialize(dataset_root: &Path, source: &Path, output: &Path, selection: &Selection, limit: usize) -> Result<()> {
    ensure!(path(dataset_root, source, selection, limit)? == output,
        "metadata source or selection changed since loading the job; rerun it");
    let parent = output.parent().context("selected metadata has no parent")?;
    if parent.exists() {
        let meta = fs::symlink_metadata(parent)?;
        ensure!(meta.is_dir() && !meta.file_type().is_symlink(),
            "selection directory is not a real directory: {}", parent.display());
    } else {
        fs::create_dir(parent).with_context(|| format!("creating selection directory {}", parent.display()))?;
    }
    ensure!(parent.canonicalize()?.starts_with(dataset_root), "selection directory escapes the dataset root");
    let delimiter = if source.extension().is_some_and(|ext| ext.eq_ignore_ascii_case("csv")) { b',' } else { b'\t' };
    let mut reader = ReaderBuilder::new().delimiter(delimiter).from_path(source)?;
    let headers = reader.headers()?.clone();
    let required = selection.require_nonempty.iter().map(|name| column(&headers, name))
        .collect::<Result<Vec<_>>>()?;
    let sort = selection.sort.iter().map(|key| Ok((column(&headers, &key.column)?, key.descending)))
        .collect::<Result<Vec<_>>>()?;
    let mut rows = Vec::<(usize, StringRecord)>::new();
    for (position, record) in reader.records().enumerate() {
        let record = record?;
        if required.iter().all(|&index| record.get(index).is_some_and(|value| !value.trim().is_empty())) {
            rows.push((position, record));
        }
    }
    ensure!(!rows.is_empty(), "no metadata rows match [selection] in {}", source.display());
    if !sort.is_empty() {
        rows.sort_unstable_by(|(a_position, a), (b_position, b)| {
            sort.iter().map(|&(index, descending)| {
                let left = a.get(index).unwrap_or("").trim();
                let right = b.get(index).unwrap_or("").trim();
                // Missing sort values always come last, independently of direction.
                match (left.is_empty(), right.is_empty()) {
                    (false, true) => Ordering::Less,
                    (true, false) => Ordering::Greater,
                    (true, true) => Ordering::Equal,
                    (false, false) => if descending { left.cmp(right).reverse() } else { left.cmp(right) },
                }
            }).find(|order| *order != Ordering::Equal).unwrap_or_else(|| a_position.cmp(b_position))
        });
    }
    let selected = rows.len().min(limit);
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    {
        let mut writer = WriterBuilder::new().has_headers(false).delimiter(delimiter).from_writer(tmp.as_file_mut());
        writer.write_record(&headers)?;
        for (_, row) in rows.iter().take(limit) { writer.write_record(row)?; }
        writer.flush()?;
    }
    ensure!(path(dataset_root, source, selection, limit)? == output,
        "metadata source changed while selecting records; retry after it stops changing");
    if let Ok(meta) = fs::symlink_metadata(output) {
        ensure!(meta.is_file() && !meta.file_type().is_symlink(),
            "selection snapshot must be a regular file: {}", output.display());
        ensure!(identical(tmp.path(), output)?,
            "selection snapshot differs from its source/config; refuse to replace {}, select a fresh job", output.display());
    } else {
        tmp.persist_noclobber(output).with_context(|| format!("publishing selection {}", output.display()))?;
    }
    eprintln!("🦎 mato: metadata selection: {} matching rows; {} selected before ingestion; snapshot {}",
        rows.len(), selected, output.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonempty_filters_then_sorts_before_limiting_and_preserves_source() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("metadata.tsv");
        let original = b"filename\tobservation_subject\tcollector_full_name\na.mzML\tLeaf\tAlice\nc.mzML\tLeaf\t\nb.mzML\tStems\tBob\nd.mzML\t\tDan\nz.mzML\tLeaf\tZoe\n";
        fs::write(&source, original).unwrap();
        let selection = Selection { require_nonempty: vec!["observation_subject".into(), "collector_full_name".into()],
            sort: vec![SortKey { column: "filename".into(), descending: true }] };
        let target = path(dir.path(), &source, &selection, 2).unwrap();
        materialize(dir.path(), &source, &target, &selection, 2).unwrap();
        let selected = fs::read_to_string(&target).unwrap();
        assert_eq!(selected, "filename\tobservation_subject\tcollector_full_name\nz.mzML\tLeaf\tZoe\nb.mzML\tStems\tBob\n");
        assert_eq!(fs::read(&source).unwrap(), original);
        materialize(dir.path(), &source, &target, &selection, 2).unwrap();
        fs::write(&source, b"filename\tobservation_subject\tcollector_full_name\nnew.mzML\tLeaf\tAlice\n").unwrap();
        assert_ne!(path(dir.path(), &source, &selection, 2).unwrap(), target);
    }

    #[test]
    fn missing_header_and_empty_selection_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("metadata.tsv");
        fs::write(&source, "filename\torgan\na.mzML\t\n").unwrap();
        let selected = Selection { require_nonempty: vec!["observation_subject".into()], sort: vec![] };
        let target = path(dir.path(), &source, &selected, 25).unwrap();
        assert!(materialize(dir.path(), &source, &target, &selected, 25).unwrap_err().to_string().contains("absent"));
        let selected = Selection { require_nonempty: vec!["organ".into()], sort: vec![] };
        let target = path(dir.path(), &source, &selected, 25).unwrap();
        assert!(materialize(dir.path(), &source, &target, &selected, 25).unwrap_err().to_string().contains("no metadata rows"));
    }
}
