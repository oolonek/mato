use std::borrow::Cow;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};

use crate::config::Config;

const TEMPLATE: &str = include_str!("../templates/run-report-v1.md");

pub(crate) struct Start {
    started: Duration,
    last_annotation_id: Option<i64>,
}

pub(crate) fn start(config: &Config) -> Start {
    let started = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let last_annotation_id = if config.db.is_file() {
        Connection::open_with_flags(&config.db, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .ok()
            .and_then(|db| {
                let exists = relation_exists(&db, "annotation_runs").ok()?;
                if !exists {
                    return Some(0);
                }
                db.query_row(
                    "SELECT COALESCE(MAX(id),0) FROM annotation_runs",
                    [],
                    |row| row.get(0),
                )
                .ok()
            })
    } else {
        Some(0)
    };
    Start {
        started,
        last_annotation_id,
    }
}

fn relation_exists(db: &Connection, name: &str) -> rusqlite::Result<bool> {
    db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name=?1 AND type IN ('table','view'))",
        [name],
        |row| row.get(0),
    )
}

fn cell(text: &str) -> Cow<'_, str> {
    if !text.contains(['&', '<', '>', '|', '\n', '\r', '`']) {
        return Cow::Borrowed(text);
    }
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '|' => escaped.push_str("&#124;"),
            '`' => escaped.push_str("&#96;"),
            '\n' => escaped.push_str("<br>"),
            '\r' => {}
            _ => escaped.push(ch),
        }
    }
    Cow::Owned(escaped)
}

fn metric(table: &mut String, name: &str, value: Option<i64>) {
    match value {
        Some(value) => writeln!(table, "| {name} | {value} |").expect("writing to String"),
        None => writeln!(table, "| {name} | not available |").expect("writing to String"),
    }
}

fn dataset_count(db: &Connection, table: &str, id: Option<i64>) -> Result<Option<i64>> {
    let Some(id) = id else {
        return Ok(None);
    };
    if !relation_exists(db, table)? {
        return Ok(None);
    }
    let sql = match table {
        "sample_rows" => "SELECT COUNT(*) FROM sample_rows WHERE dataset_id=?1",
        "spectrum_files" => "SELECT COUNT(*) FROM spectrum_files WHERE dataset_id=?1",
        "processed_spectra" => "SELECT COUNT(*) FROM processed_spectra WHERE dataset_id=?1",
        _ => bail!("unknown dataset relation {table}"),
    };
    Ok(Some(db.query_row(sql, [id], |row| row.get(0))?))
}

struct Data {
    metrics: String,
    failures: String,
}

fn collect(config: &Config, start: &Start) -> Result<Data> {
    let mut metrics = String::from("| Metric | Value |\n| --- | ---: |\n");
    let mut failures = String::new();
    if !config.db.is_file() {
        for name in [
            "Ingested sample records",
            "Spectrum files",
            "Registered spectra",
            "Current hit files",
            "Current no-hit files",
            "Current annotation hits",
            "Outstanding failed files",
            "Chemical lookups (database-wide)",
        ] {
            metric(&mut metrics, name, None);
        }
        return Ok(Data {
            metrics,
            failures: "Not available: database has not been created.\n".to_owned(),
        });
    }
    let db = Connection::open_with_flags(&config.db, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("opening {} read-only for report", config.db.display()))?;
    let dataset_id = if relation_exists(&db, "datasets")? {
        db.query_row(
            "SELECT id FROM datasets WHERE doi=?1 AND root=?2 AND metadata_path=?3",
            params![
                config.doi,
                config
                    .dataset_root
                    .to_str()
                    .context("dataset root is not UTF-8")?,
                config
                    .metadata
                    .to_str()
                    .context("metadata path is not UTF-8")?
            ],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
    } else {
        None
    };
    metric(
        &mut metrics,
        "Ingested sample records",
        dataset_count(&db, "sample_rows", dataset_id)?,
    );
    metric(
        &mut metrics,
        "Spectrum files",
        dataset_count(&db, "spectrum_files", dataset_id)?,
    );
    metric(
        &mut metrics,
        "Registered spectra",
        dataset_count(&db, "processed_spectra", dataset_id)?,
    );
    if let (Some(id), true) = (dataset_id, relation_exists(&db, "annotation_current_runs")?) {
        for (label, status) in [
            ("Current hit files", "hits"),
            ("Current no-hit files", "no_hits"),
        ] {
            let count = db.query_row(
                "SELECT COUNT(*) FROM annotation_current_runs WHERE dataset_id=?1 AND status=?2",
                params![id, status],
                |row| row.get(0),
            )?;
            metric(&mut metrics, label, Some(count));
        }
        let hits = db.query_row("SELECT COALESCE(SUM(hit_count),0) FROM annotation_current_runs WHERE dataset_id=?1 AND status='hits'", [id], |row| row.get(0))?;
        metric(&mut metrics, "Current annotation hits", Some(hits));
    } else {
        for label in [
            "Current hit files",
            "Current no-hit files",
            "Current annotation hits",
        ] {
            metric(&mut metrics, label, None);
        }
    }
    let lookups = if relation_exists(&db, "chemical_lookups")? {
        Some(
            db.query_row("SELECT COUNT(*) FROM chemical_lookups", [], |row| {
                row.get(0)
            })?,
        )
    } else {
        None
    };
    metric(&mut metrics, "Chemical lookups (database-wide)", lookups);

    if relation_exists(&db, "annotation_runs")? {
        writeln!(failures, "| Dataset ID | Filename | Taxon | Wikidata QID | Reason | New this run |\n| ---: | --- | --- | --- | --- | --- |").expect("writing to String");
        let mut query = db.prepare(
            "SELECT r.id,r.dataset_id,r.filename,r.taxon_name,r.taxon_qid,r.error
             FROM annotation_runs r WHERE r.status='error'
               AND r.id=(SELECT MAX(id) FROM annotation_runs WHERE dataset_id=r.dataset_id AND filename=r.filename)
               AND (r.dataset_id=?1 OR r.id>?2)
             ORDER BY r.dataset_id,r.filename",
        )?;
        let mut rows = query.query(params![
            dataset_id,
            start.last_annotation_id.unwrap_or(i64::MAX)
        ])?;
        let mut count = 0;
        let mut dataset_errors = 0;
        while let Some(row) = rows.next()? {
            let id: i64 = row.get(0)?;
            let dataset: i64 = row.get(1)?;
            let filename: String = row.get(2)?;
            if Some(dataset) == dataset_id {
                dataset_errors += 1;
            }
            let taxon: Option<String> = row.get(3)?;
            let qid: Option<String> = row.get(4)?;
            let reason: Option<String> = row.get(5)?;
            let new = start
                .last_annotation_id
                .map(|last| if id > last { "yes" } else { "no" })
                .unwrap_or("unknown");
            writeln!(
                failures,
                "| {dataset} | {} | {} | {} | {} | {new} |",
                cell(&filename),
                cell(taxon.as_deref().unwrap_or("")),
                cell(qid.as_deref().unwrap_or("")),
                cell(reason.as_deref().unwrap_or(""))
            )
            .expect("writing to String");
            count += 1;
        }
        metric(
            &mut metrics,
            "Outstanding failed files",
            dataset_id.map(|_| dataset_errors),
        );
        if count == 0 {
            failures.clear();
            failures.push_str("No outstanding failed files.\n");
        }
    } else {
        failures.push_str("Not available: annotation has not run.\n");
        metric(&mut metrics, "Outstanding failed files", None);
    }
    Ok(Data { metrics, failures })
}

fn quote_shell(value: &Path) -> String {
    let mut result = String::from("'");
    for ch in value.to_string_lossy().chars() {
        if ch == '\'' {
            result.push_str("'\\''");
        } else {
            result.push(ch);
        }
    }
    result.push('\'');
    result
}

fn render(writer: &mut impl std::io::Write, fields: &[(&str, &str)]) -> Result<()> {
    let mut remaining = TEMPLATE;
    while let Some(start) = remaining.find("{{") {
        writer.write_all(remaining[..start].as_bytes())?;
        let after = &remaining[start + 2..];
        let end = after
            .find("}}")
            .context("unterminated report template token")?;
        let token = &after[..end];
        let value = fields
            .iter()
            .find(|(name, _)| *name == token)
            .with_context(|| format!("unknown report template token {token}"))?
            .1;
        writer.write_all(value.as_bytes())?;
        remaining = &after[end + 2..];
    }
    writer.write_all(remaining.as_bytes())?;
    Ok(())
}

pub(crate) fn write(
    config: &Config,
    top_n: Option<usize>,
    start: &Start,
    status: &str,
    error: Option<&anyhow::Error>,
) -> Result<PathBuf> {
    let report_dir = config
        .db
        .parent()
        .context("database has no parent directory")?
        .join("reports");
    fs::create_dir_all(&report_dir)
        .with_context(|| format!("creating report directory {}", report_dir.display()))?;
    let data = match collect(config, start) {
        Ok(data) => data,
        Err(problem) => Data {
            metrics: format!(
                "Database inspection unavailable: {}\n",
                cell(&format!("{problem:#}"))
            ),
            failures: "Not available: database could not be inspected.\n".to_owned(),
        },
    };
    let mut stages = String::new();
    for (index, stage) in config.stages.iter().enumerate() {
        if index > 0 {
            stages.push_str(", ");
        }
        stages.push_str(stage.name());
    }
    let mut files = String::new();
    for (name, path) in [
        ("Pipeline TOML", &config.path),
        ("Dataset", &config.dataset_root),
        ("Metadata TSV", &config.metadata),
        ("Raw spectra", &config.ms_dir),
        ("SQLite database", &config.db),
        ("Picoto output", &config.output_dir),
        ("Spectral library", &config.library),
        ("LOTUS CSV", &config.lotus_csv),
        ("Matcher settings", &config.settings),
        ("Shared Wikidata taxon index", &config.taxon_index),
        ("Shared Wikidata chemical index", &config.chemical_index),
    ] {
        writeln!(files, "- {name}: {}", cell(&path.display().to_string()))
            .expect("writing to String");
    }
    let manifest = config.flock_root.join("mato/Cargo.toml");
    let mut commands = String::new();
    for (table, limit) in [
        (None, 0),
        (Some("annotation_current_runs"), 20),
        (Some("annotation_runs"), 200),
    ] {
        write!(
            commands,
            "cargo run --release --manifest-path {} -- ",
            quote_shell(&manifest)
        )
        .expect("writing to String");
        if let Some(table) = table {
            write!(
                commands,
                "show {table} --config {} --limit {limit}",
                quote_shell(&config.path)
            )
            .expect("writing to String");
        } else {
            write!(commands, "inspect --config {}", quote_shell(&config.path))
                .expect("writing to String");
        }
        if let Some(n) = top_n {
            write!(commands, " --topN {n}").expect("writing to String");
        }
        commands.push('\n');
    }
    let failure = match error {
        Some(error) => format!("> {}\n", cell(&format!("{error:#}"))),
        None => "None.\n".to_owned(),
    };
    let doi = cell(&config.doi);
    let started = start.started.as_secs().to_string();
    let finished = SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_secs()
        .to_string();
    let selection = top_n
        .map(|value| format!("first {value} rows"))
        .unwrap_or_else(|| "all rows".to_owned());
    let name_base = format!("run-{}-{}", start.started.as_millis(), std::process::id());
    for sequence in 0..100 {
        let run_id = format!("{name_base}-{sequence}");
        let destination = report_dir.join(format!("{run_id}.md"));
        let mut temporary = tempfile::Builder::new()
            .prefix(".mato-report-")
            .tempfile_in(&report_dir)?;
        render(
            temporary.as_file_mut(),
            &[
                ("RUN_ID", &run_id),
                ("STATUS", status),
                ("DOI", &doi),
                ("STARTED", &started),
                ("FINISHED", &finished),
                ("STAGES", &stages),
                ("SELECTION", &selection),
                ("METRICS", &data.metrics),
                ("FILES", &files),
                ("FAILURES", &data.failures),
                ("COMMANDS", &commands),
                ("ERROR", &failure),
            ],
        )?;
        temporary.as_file_mut().sync_all()?;
        match temporary.persist_noclobber(&destination) {
            Ok(_) => return Ok(destination),
            Err(problem) if problem.error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(problem) => {
                return Err(problem.error)
                    .with_context(|| format!("saving report {}", destination.display()));
            }
        }
    }
    bail!(
        "unable to choose a unique run report filename in {}",
        report_dir.display()
    )
}
