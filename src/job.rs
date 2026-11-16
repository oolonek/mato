use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags};

use crate::config::Config;

fn database_exists(path: &Path) -> Result<bool> {
    match fs::metadata(path) {
        Ok(metadata) => {
            ensure!(
                metadata.is_file(),
                "database is not a regular file: {}",
                path.display()
            );
            Ok(true)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("checking database {}", path.display())),
    }
}

fn table_exists(db: &Connection, name: &str) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [name],
        |row| row.get::<_, i64>(0),
    )? != 0)
}

pub(crate) fn inspect(config: &Config) -> Result<()> {
    println!("config: {}", config.path.display());
    if let Some(n) = config.top_n {
        println!("selection: first {n} metadata rows (isolated top-N job)");
    }
    println!("dataset: {}", config.dataset_root.display());
    println!("DOI: {}", config.doi);
    println!("metadata: {}", config.metadata.display());
    print!("stages: ");
    for (index, stage) in config.stages.iter().enumerate() {
        if index > 0 {
            print!(", ");
        }
        print!("{}", stage.name());
    }
    println!();
    let db = &config.db;
    if database_exists(db)? {
        println!("database: {} (present)", db.display());
        let connection = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("opening database {} read-only", db.display()))?;
        if table_exists(&connection, "datasets")? {
            let mut query = connection.prepare("SELECT id, doi FROM datasets ORDER BY id")?;
            let rows = query.query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (id, doi) = row?;
                println!("stored dataset: {id} {doi}");
            }
        } else {
            println!("datasets: not yet created");
        }
        for (table, sql) in [
            ("sample_rows", "SELECT count(*) FROM sample_rows"),
            ("spectrum_files", "SELECT count(*) FROM spectrum_files"),
            ("annotation_runs", "SELECT count(*) FROM annotation_runs"),
        ] {
            if table_exists(&connection, table)? {
                let count: i64 = connection.query_row(sql, [], |row| row.get(0))?;
                println!("{table}: {count}{}", if table == "annotation_runs" { " historical attempts" } else { "" });
            } else {
                println!("{table}: not yet created");
            }
        }
        let has_current: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='annotation_current_runs' AND type='view')",
            [], |row| row.get(0))?;
        if has_current {
            let (files, errors): (i64, i64) = connection.query_row(
                "SELECT COUNT(*),COALESCE(SUM(status='error'),0) FROM annotation_current_runs",
                [], |row| Ok((row.get(0)?, row.get(1)?)))?;
            println!("annotation_current_runs: {files} current files ({errors} errors)");
        } else {
            println!("annotation_current_runs: not yet created");
        }
    } else {
        println!("database: {} (missing)", db.display());
    }
    println!(
        "Picoto: {} ({})",
        config.output_dir.display(),
        if config.output_dir.exists() {
            "present"
        } else {
            "missing"
        }
    );
    println!("Wikidata indexes config: {}", config.indexes_config.display());
    println!("Wikidata taxon index: {} ({})", config.taxon_index.display(),
        if config.taxon_index.is_file() { "present" } else { "missing" });
    println!("Wikidata chemical index: {} ({})", config.chemical_index.display(),
        if config.chemical_index.is_file() { "present" } else { "missing" });
    Ok(())
}

pub(crate) fn require_db(config: &Config) -> Result<()> {
    ensure!(
        database_exists(&config.db)?,
        "database does not exist: {}. Run `mato run --config {}{}` with the ingest stage selected before inspecting annotations",
        config.db.display(),
        config.path.display(),
        config.top_n.map(|n| format!(" --topN {n}")).unwrap_or_default(),
    );
    Ok(())
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = OsString::from(path.as_os_str());
    name.push(suffix);
    PathBuf::from(name)
}

fn remove_file_if_present(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("removing {}", path.display())),
    }
}

fn resolved_path(path: &Path) -> Result<PathBuf> {
    match path.canonicalize() {
        Ok(resolved) => Ok(resolved),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let parent = path.parent().context("reset target has no parent")?;
            let name = path.file_name().context("reset target has no filename")?;
            Ok(resolved_path(parent)?.join(name))
        }
        Err(error) => {
            Err(error).with_context(|| format!("resolving reset target {}", path.display()))
        }
    }
}

pub(crate) fn reset(config: &Config) -> Result<()> {
    let db = &config.db;
    let output = &config.output_dir;
    let resolved_db = resolved_path(db)?;
    let resolved_output = resolved_path(output)?;
    ensure!(
        !resolved_db.starts_with(&config.dataset_root)
            && !resolved_db.starts_with(&config.flock_root),
        "unsafe database path (inside source dataset or code): {}",
        db.display(),
    );
    ensure!(
        !resolved_output.starts_with(&config.dataset_root)
            && !config.dataset_root.starts_with(&resolved_output)
            && !resolved_output.starts_with(&config.flock_root)
            && !config.flock_root.starts_with(&resolved_output)
            && !config.path.starts_with(&resolved_output)
            && !resolved_db.starts_with(&resolved_output),
        "unsafe Picoto output directory: {}",
        output.display(),
    );
    if let Ok(metadata) = fs::symlink_metadata(db) {
        ensure!(
            !metadata.file_type().is_symlink() && metadata.is_file(),
            "database is not a regular file: {}",
            db.display()
        );
    }
    if let Ok(metadata) = fs::symlink_metadata(output) {
        ensure!(
            !metadata.file_type().is_symlink() && metadata.is_dir(),
            "Picoto output is not a directory: {}",
            output.display()
        );
    }
    // An exclusive transaction catches most concurrent SQLite users before we remove files.
    // Stop all species processes first: SQLite WAL readers do not necessarily block this lock.
    if db.exists() {
        let connection = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_WRITE)
            .with_context(|| format!("opening {} for reset", db.display()))?;
        connection
            .execute_batch("PRAGMA busy_timeout=0; BEGIN EXCLUSIVE; ROLLBACK;")
            .with_context(|| format!("database is in use: {}", db.display()))?;
    }
    println!("Deleting database: {}", db.display());
    println!("Deleting Picoto output: {}", output.display());
    remove_file_if_present(db)?;
    for suffix in ["-wal", "-shm", "-journal"] {
        remove_file_if_present(&sidecar(db, suffix))?;
    }
    if output.exists() {
        fs::remove_dir_all(output)
            .with_context(|| format!("removing Picoto output {}", output.display()))?;
    }
    println!(
        "Job reset; run `mato run --config {}{}` with the full stage list to start fresh",
        config.path.display(),
        config.top_n.map(|n| format!(" --topN {n}")).unwrap_or_default(),
    );
    Ok(())
}
