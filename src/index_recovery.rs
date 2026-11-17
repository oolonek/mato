//! Explicitly recover a shared index after an interrupted SQLite refresh.
//! Normal reconciliation continues to open these indexes read-only.
use std::fs;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags};

use crate::IndexKind;
use crate::config::SharedIndexes;

fn recover_one(path: &Path, kind: &str, snapshot: &str, rows: &str) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("{kind} index does not exist: {}", path.display()))?;
    ensure!(metadata.file_type().is_file(), "{kind} index is not a regular file: {}", path.display());
    // Opening without CREATE and taking a write reservation lets SQLite, not
    // file deletion, roll back a hot journal. It also refuses an active writer.
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_WRITE)
        .with_context(|| format!("open {kind} index read-write to recover SQLite journal: {}", path.display()))?;
    db.busy_timeout(Duration::from_secs(5))?;
    db.execute_batch("BEGIN IMMEDIATE")
        .with_context(|| format!("lock {kind} index for recovery; stop any active index refresh first"))?;
    let check: String = db.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
    ensure!(check == "ok", "{kind} index failed SQLite quick_check: {check}");
    let (generation, expected): (i64, i64) = db.query_row(snapshot, [], |row| Ok((row.get(0)?, row.get(1)?)))
        .with_context(|| format!("{kind} index has no completed snapshot; refresh it instead"))?;
    ensure!(generation > 0 && expected > 0, "{kind} index has no completed generation or rows");
    let actual: i64 = db.query_row(rows, [], |row| row.get(0))?;
    ensure!(expected == actual, "{kind} snapshot expects {expected} rows but has {actual}; index was not replaced");
    db.execute_batch("ROLLBACK")?; // No snapshot data was changed by recovery.
    println!("Recovered {kind} index: {} (generation {generation}, {actual} rows)", path.display());
    Ok(())
}

pub(crate) fn run(config: &Path, only: Option<IndexKind>) -> Result<()> {
    let indexes = SharedIndexes::load(config)?;
    for (kind, name, path, snapshot, rows) in [
        (IndexKind::Taxon, "taxon", &indexes.taxon_index,
         "SELECT generation,names FROM wikidata_taxon_snapshot WHERE id=1",
         "SELECT COUNT(*) FROM wikidata_taxon_names"),
        (IndexKind::Chemical, "chemical", &indexes.chemical_index,
         "SELECT generation,pairs FROM wikidata_chemical_snapshot WHERE id=1",
         "SELECT COUNT(*) FROM wikidata_chemical_index"),
    ] {
        if only.is_none_or(|chosen| chosen == kind) {
            recover_one(path, name, snapshot, rows)?;
        }
    }
    Ok(())
}
