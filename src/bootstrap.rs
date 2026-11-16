use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

use crate::config::{BootstrapPaths, Config, SharedIndexes, Stage};

const DEFAULT_QLEVER: &str = "https://qlever.dev/api/wikidata";
const FULL_TOY_PIPELINE: [Stage; 7] = [
    Stage::Validate,
    Stage::Ingest,
    Stage::Enrich,
    Stage::Picoto,
    Stage::Register,
    Stage::Annotate,
    Stage::ChemicalEnrich,
];

struct Index<'a> {
    name: &'static str,
    path: &'a Path,
    command: &'static str,
    snapshot_query: &'static str,
    count_query: &'static str,
    endpoint: &'a str,
}

fn complete(index: &Index<'_>) -> Result<bool> {
    if !index.path.exists() {
        return Ok(false);
    }
    // SQLite leaves a zero-byte file if its first transactional refresh rolls
    // back. It contains no user data and can safely be populated on retry.
    if fs::metadata(index.path)?.len() == 0 {
        return Ok(false);
    }
    let check = || -> Result<()> {
        let db = Connection::open_with_flags(index.path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .with_context(|| format!("open {}", index.path.display()))?;
        let integrity: String = db.query_row("PRAGMA quick_check", [], |row| row.get(0))?;
        ensure!(integrity == "ok", "SQLite quick_check: {integrity}");
        let (generation, expected, endpoint): (i64, i64, String) = db
            .query_row(index.snapshot_query, [], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .optional()?
            .context("snapshot record absent")?;
        ensure!(generation > 0 && expected > 0, "snapshot has no completed generation or rows");
        ensure!(endpoint == index.endpoint, "snapshot endpoint {endpoint} differs from configured {}", index.endpoint);
        let actual: i64 = db.query_row(index.count_query, [], |row| row.get(0))?;
        ensure!(actual == expected, "snapshot records {expected} rows, found {actual}");
        Ok(())
    };
    check().with_context(|| format!(
        "incomplete {} index at {}; it was not overwritten; inspect the file and refresh it deliberately",
        index.name,
        index.path.display()
    ))?;
    eprintln!("🦎 mato: verified existing {} index: {}", index.name, index.path.display());
    Ok(true)
}

fn indexes<'a>(paths: &'a BootstrapPaths, shared: &'a SharedIndexes) -> [Index<'a>; 2] {
    [
        Index {
            name: "taxon",
            path: &paths.taxon_index,
            command: "taxon-index",
            snapshot_query: "SELECT generation, names, endpoint FROM wikidata_taxon_snapshot WHERE id=1",
            count_query: "SELECT COUNT(*) FROM wikidata_taxon_names",
            endpoint: shared.taxon_endpoint.as_deref().unwrap_or(DEFAULT_QLEVER),
        },
        Index {
            name: "chemical",
            path: &paths.chemical_index,
            command: "chemical-index",
            snapshot_query: "SELECT generation, pairs, endpoint FROM wikidata_chemical_snapshot WHERE id=1",
            count_query: "SELECT COUNT(*) FROM wikidata_chemical_index",
            endpoint: shared.chemical_endpoint.as_deref().unwrap_or(DEFAULT_QLEVER),
        },
    ]
}

fn refresh_missing(paths: &BootstrapPaths, indexes: &[Index<'_>; 2], complete_before: [bool; 2]) -> Result<()> {
    if complete_before.iter().all(|ready| *ready) {
        return Ok(());
    }
    let cunaguaro = crate::binary(&paths.flock_root, "cunaguaro")?;
    for (index, ready) in indexes.iter().zip(complete_before) {
        if ready {
            continue;
        }
        let mut child = Command::new(&cunaguaro);
        child.arg(index.command).arg("--index").arg(index.path);
        if index.endpoint != DEFAULT_QLEVER {
            child.arg("--endpoint").arg(index.endpoint);
        }
        crate::status(child, &format!("building shared {} index", index.name))?;
        ensure!(complete(index)?, "{} index was not created", index.name);
    }
    Ok(())
}

fn dataset_id(db: &Path, doi: &str, root: &Path) -> Result<i64> {
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("read ingested database {}", db.display()))?;
    let id = conn.query_row(
        "SELECT id FROM datasets WHERE doi=?1 AND root=?2",
        params![doi, root.to_str().context("non-UTF-8 dataset root")?],
        |row| row.get(0),
    ).optional()?;
    id.with_context(|| format!("dataset {doi} at {} was not ingested", root.display()))
}

fn export(paths: &BootstrapPaths, config: &Config) -> Result<PathBuf> {
    ensure!(
        !paths.graph_dir.starts_with(&config.dataset_root)
            && !paths.graph_dir.starts_with(&config.flock_root),
        "KG output must not be inside the source dataset or code checkout"
    );
    let id = dataset_id(&config.db, &config.doi, &config.dataset_root)?;
    fs::create_dir_all(&paths.graph_dir)
        .with_context(|| format!("creating KG directory {}", paths.graph_dir.display()))?;
    let millis = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let graph = paths.graph_dir.join(format!("graph-{millis}-{}.nq", std::process::id()));
    let peregrino = crate::binary(&config.flock_root, "peregrino")?;
    let mut command = Command::new(&peregrino);
    command.arg("kg").arg("export")
        .arg("--db").arg(&config.db)
        .arg("--dataset-id").arg(id.to_string())
        .arg("--out").arg(&graph);
    crate::status(command, &format!("exporting KG to {}", graph.display()))?;
    let mut command = Command::new(&peregrino);
    command.arg("kg").arg("validate").arg("--graph").arg(&graph);
    crate::status(command, &format!("validating KG {}", graph.display()))?;
    Ok(graph)
}

pub(crate) fn run(path: &Path) -> Result<()> {
    let paths = crate::config::bootstrap_paths(path)?;
    ensure!(paths.doi == "10.5281/zenodo.10018264", "bootstrap currently supports only the Zenodo ENPKG toy DOI");
    let shared = SharedIndexes::load(path)?;
    let indexes = indexes(&paths, &shared);
    // Reject a broken existing snapshot before downloading anything or creating another database.
    let ready = [complete(&indexes[0])?, complete(&indexes[1])?];
    crate::bootstrap_artifacts::prepare(&paths)?;
    let config = Config::load(path, None)?;
    ensure!(config.stages == FULL_TOY_PIPELINE, "bootstrap requires the complete toy pipeline stages in flock order");
    refresh_missing(&paths, &indexes, ready)?;
    crate::run_reported(path, false, None)?;
    let graph = export(&paths, &config)?;
    println!("Validated KG: {}", graph.display());
    println!("Database: {}", config.db.display());
    println!(
        "Query: cargo run --release --manifest-path '{}' -- kg query --graph '{}' --query '{}'",
        paths.flock_root.join("peregrino/Cargo.toml").display(),
        graph.display(),
        paths.flock_root.join("peregrino/kg/taxonomic-evidence.rq").display(),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interrupted_first_refresh_can_retry_without_deleting_empty_cache() {
        let temporary = tempfile::tempdir().unwrap();
        let file = temporary.path().join("chemicals.sqlite");
        fs::write(&file, []).unwrap();
        let index = Index {
            name: "chemical",
            path: &file,
            command: "chemical-index",
            snapshot_query: "SELECT generation, pairs, endpoint FROM wikidata_chemical_snapshot WHERE id=1",
            count_query: "SELECT COUNT(*) FROM wikidata_chemical_index",
            endpoint: DEFAULT_QLEVER,
        };
        assert!(!complete(&index).unwrap());
        assert_eq!(fs::metadata(file).unwrap().len(), 0);
    }
}
