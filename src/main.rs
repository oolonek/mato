mod config;
mod job;
mod report;

use std::collections::{BTreeSet, HashMap};
use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::io::Write;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;

use anyhow::{Context, Result, bail, ensure};
use clap::{Parser, Subcommand, ValueEnum};
use indicatif::{ProgressBar, ProgressStyle};
use walkdir::WalkDir;
use rusqlite::{Connection, OpenFlags, params};

use crate::config::{Config, SharedIndexes, Stage};

#[derive(Parser)]
#[command(about = "Run the configured flock pipeline using the existing species CLIs")]
struct Cli {
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    /// Run the stages selected in a TOML file in flock order.
    Run {
        #[arg(long)]
        config: PathBuf,
        /// Run the first N metadata rows in an isolated top-N job configured in this TOML.
        #[arg(long = "topN")]
        top_n: Option<usize>,
        /// Stream registration filenames and Cunaguaro per-name results.
        #[arg(long)]
        verbose: bool,
    },
    /// Refresh the shared Wikidata indexes configured once for all jobs.
    Indexes {
        #[command(subcommand)]
        action: IndexAction,
    },
    /// Inspect the configured job paths and database without creating it.
    Inspect {
        #[arg(long)]
        config: PathBuf,
        /// Inspect the isolated job for the first N metadata rows.
        #[arg(long = "topN")]
        top_n: Option<usize>,
    },
    /// Delete this job's database and Picoto outputs permanently.
    Reset {
        #[arg(long)]
        config: PathBuf,
        /// Reset only the isolated job for the first N metadata rows.
        #[arg(long = "topN")]
        top_n: Option<usize>,
        /// Confirm deletion of the configured database and Picoto output directory.
        #[arg(long, required = true)]
        yes: bool,
    },
    /// Preview a table or view from the configured job database.
    Show {
        table: String,
        #[arg(long)]
        config: PathBuf,
        /// Preview the isolated job for the first N metadata rows.
        #[arg(long = "topN")]
        top_n: Option<usize>,
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
}

#[derive(Subcommand)]
enum IndexAction {
    /// Download both QLever snapshots, or only one of them.
    Refresh {
        /// Alternate shared-index TOML; defaults to mato/config/wikidata.toml.
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        only: Option<IndexKind>,
    },
}

#[derive(Clone, Copy, Eq, PartialEq, ValueEnum)]
enum IndexKind {
    Taxon,
    Chemical,
}

fn child_outcome(mut command: Command, description: &str) -> Result<std::process::ExitStatus> {
    command
        .status()
        .with_context(|| format!("starting {description}"))
}

fn status(command: Command, description: &str) -> Result<()> {
    let outcome = child_outcome(command, description)?;
    ensure!(
        outcome.success(),
        "{description} failed ({outcome}); stopping pipeline"
    );
    Ok(())
}

fn binary(flock_root: &Path, species: &'static str) -> Result<PathBuf> {
    let repo = flock_root.join(species);
    let manifest = repo.join("Cargo.toml");
    let target = match env::var_os("CARGO_TARGET_DIR") {
        Some(path) if Path::new(&path).is_absolute() => PathBuf::from(path),
        Some(path) => env::current_dir()?.join(path),
        None => repo.join("target"),
    };
    let mut build = Command::new(env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo")));
    build
        .current_dir(flock_root)
        .arg("build")
        .arg("--release")
        .arg("--manifest-path")
        .arg(&manifest)
        .arg("--bin")
        .arg(species)
        .arg("--target-dir")
        .arg(&target);
    status(build, &format!("building {species}"))?;
    let binary = target
        .join("release")
        .join(format!("{species}{}", env::consts::EXE_SUFFIX));
    ensure!(
        binary.is_file(),
        "built {species} executable missing: {}",
        binary.display()
    );
    Ok(binary)
}

fn refresh_indexes(config: Option<PathBuf>, only: Option<IndexKind>) -> Result<()> {
    let config = config.unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("config/wikidata.toml"));
    let indexes = SharedIndexes::load(&config)?;
    let flock_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent().context("Mato source tree has no parent")?;
    let executable = binary(flock_root, "cunaguaro")?;
    for (kind, command, path, endpoint) in [
        (IndexKind::Taxon, "taxon-index", &indexes.taxon_index, indexes.taxon_endpoint.as_deref()),
        (IndexKind::Chemical, "chemical-index", &indexes.chemical_index, indexes.chemical_endpoint.as_deref()),
    ] {
        if only.is_some_and(|selected| selected != kind) {
            continue;
        }
        let mut child = Command::new(&executable);
        child.arg(command).arg("--index").arg(path);
        if let Some(endpoint) = endpoint {
            child.arg("--endpoint").arg(endpoint);
        }
        status(child, &format!("refreshing shared {command} at {}", path.display()))?;
    }
    Ok(())
}

fn limited_db(config: &Config, max_rows: usize) -> Result<Option<Connection>> {
    ensure!(max_rows > 0, "--topN must be positive");
    if !config.db.exists() {
        return Ok(None);
    }
    let db = Connection::open_with_flags(&config.db, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("opening limited-run database {}", config.db.display()))?;
    let datasets = {
        let mut query = db.prepare("SELECT root, metadata_path FROM datasets")?;
        query.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    ensure!(
        datasets.len() <= 1
            && datasets.iter().all(|(root, metadata)| {
                root == config.dataset_root.to_str().unwrap_or("")
                    && metadata == config.metadata.to_str().unwrap_or("")
            }),
        "--topN requires an isolated database for this dataset and metadata"
    );
    let rows: i64 = db.query_row("SELECT COUNT(*) FROM sample_rows", [], |row| row.get(0))?;
    ensure!(
        rows <= i64::try_from(max_rows).context("--topN exceeds SQLite row range")?,
        "existing top-N database has {rows} sample rows; reset only this top-N job before changing its dataset"
    );
    Ok(Some(db))
}

fn limited_files(config: &Config, max_rows: usize) -> Result<Vec<PathBuf>> {
    let db = limited_db(config, max_rows)?
        .context("--topN needs an ingested database; run the ingest stage first")?;
    let mut query = db.prepare(
        "SELECT s.relative_path FROM spectrum_files s JOIN datasets d ON d.id=s.dataset_id \
         WHERE d.root=?1 AND d.metadata_path=?2 ORDER BY s.row_number, s.filename",
    )?;
    let paths = query.query_map(
        params![config.dataset_root.to_str().context("non-UTF-8 dataset root")?,
                config.metadata.to_str().context("non-UTF-8 metadata path")?],
        |row| row.get::<_, String>(0),
    )?;
    let mut files = Vec::new();
    let mut outputs = BTreeSet::new();
    for path in paths {
        let relative = PathBuf::from(path?);
        ensure!(!relative.is_absolute(), "registered raw path must be relative: {}", relative.display());
        let raw = config.dataset_root.join(&relative).canonicalize()
            .with_context(|| format!("registered raw file {}", relative.display()))?;
        ensure!(raw.starts_with(&config.ms_dir) && raw.is_file(),
            "registered raw file is outside spectrum directory or missing: {}", raw.display());
        let rel = raw.strip_prefix(&config.ms_dir)?;
        ensure!(rel.extension().and_then(|ext| ext.to_str()).is_some_and(|ext|
            ext.eq_ignore_ascii_case("mzml") || ext.eq_ignore_ascii_case("mzxml")),
            "picoto cannot process registered raw file: {}", raw.display());
        ensure!(rel.file_name().and_then(|name| name.to_str()).is_some(),
            "non-UTF-8 raw filename: {}", raw.display());
        let output = rel.with_extension("");
        ensure!(outputs.insert(output), "raw files would overwrite the same Picoto outputs: {}", raw.display());
        files.push(raw);
    }
    ensure!(!files.is_empty(), "no selected raw files in {}", config.db.display());
    Ok(files)
}

fn raw_files(config: &Config, max_rows: Option<usize>) -> Result<Vec<PathBuf>> {
    if let Some(max_rows) = max_rows {
        return limited_files(config, max_rows);
    }
    let mut files = Vec::new();
    let mut outputs = BTreeSet::new();
    for entry in WalkDir::new(&config.ms_dir) {
        let entry = entry.with_context(|| format!("walking {}", config.ms_dir.display()))?;
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let Some(extension) = path.extension().and_then(|ext| ext.to_str()) else {
            continue;
        };
        if extension.eq_ignore_ascii_case("mzdata")
            || extension.eq_ignore_ascii_case("gz")
                && path
                    .file_stem()
                    .and_then(|stem| Path::new(stem).extension())
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| {
                        ext.eq_ignore_ascii_case("mzml")
                            || ext.eq_ignore_ascii_case("mzxml")
                            || ext.eq_ignore_ascii_case("mzdata")
                    })
        {
            bail!(
                "picoto cannot process compressed or mzData raw file: {}",
                path.display()
            );
        }
        if !extension.eq_ignore_ascii_case("mzml") && !extension.eq_ignore_ascii_case("mzxml") {
            continue;
        }
        let relative = path.strip_prefix(&config.ms_dir)?;
        let stem = relative.file_stem().context("raw file has no stem")?;
        let output = relative
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join(stem);
        ensure!(
            outputs.insert(output),
            "raw files would overwrite the same Picoto outputs: {}",
            path.display()
        );
        ensure!(
            relative
                .file_name()
                .and_then(|name| name.to_str())
                .is_some(),
            "non-UTF-8 raw filename: {}",
            path.display()
        );
        files.push(path.to_path_buf());
    }
    files.sort_unstable();
    ensure!(
        !files.is_empty(),
        "no mzML/mzXML raw spectra in {}",
        config.ms_dir.display()
    );
    Ok(files)
}

fn register(config: &Config, binary: &Path, files: &[PathBuf], verbose: bool) -> Result<()> {
    let mut outputs = Vec::with_capacity(files.len());
    for path in files {
        let relative = path.strip_prefix(&config.ms_dir)?;
        let folder = config
            .output_dir
            .join(relative.parent().unwrap_or_else(|| Path::new("")));
        let stem = relative
            .file_stem()
            .and_then(|s| s.to_str())
            .context("non-UTF-8 raw stem")?;
        let ms2 = folder.join(format!("{stem}_ms2.mgf"));
        let audit = folder.join(format!("{stem}_features.tsv"));
        let paired = folder.join(format!("{stem}_sirius.mgf"));
        for output in [&ms2, &audit, &paired] {
            ensure!(
                output.is_file(),
                "Picoto output missing: {}",
                output.display()
            );
        }
        outputs.push((path, ms2, audit, paired));
    }
    let progress = ProgressBar::new(outputs.len() as u64);
    progress.set_style(ProgressStyle::with_template(&format!(
        "{} peregrino {{spinner:.cyan}} [{{elapsed_precise}}] [{{bar:40.cyan/blue}}] {{pos}}/{{len}} registered {{msg}}",
        Stage::Register.emoji()
    ))?);
    progress.enable_steady_tick(std::time::Duration::from_millis(120));
    let workers = config.peregrino_jobs.min(outputs.len());
    thread::scope(|scope| -> Result<()> {
        let (jobs_sender, jobs_receiver) =
            mpsc::sync_channel::<(&PathBuf, PathBuf, PathBuf, PathBuf)>(workers);
        let jobs_receiver = Arc::new(Mutex::new(jobs_receiver));
        let (results_sender, results_receiver) = mpsc::channel();
        for _ in 0..workers {
            let jobs_receiver = Arc::clone(&jobs_receiver);
            let results_sender = results_sender.clone();
            scope.spawn(move || {
                loop {
                    let job = jobs_receiver
                        .lock()
                        .expect("registration queue poisoned")
                        .recv();
                    let Ok((raw, ms2, audit, paired)) = job else {
                        break;
                    };
                    let result = (|| -> Result<()> {
                        let name = raw.file_name().context("raw file has no basename")?;
                        let mut command = Command::new(binary);
                        command
                            .arg("register")
                            .arg("--db")
                            .arg(&config.db)
                            .arg("--dataset")
                            .arg(&config.dataset_root)
                            .arg("--raw")
                            .arg(name)
                            .arg("--ms2")
                            .arg(&ms2)
                            .arg("--features")
                            .arg(&audit)
                            .arg("--paired")
                            .arg(&paired)
                            .stdout(Stdio::null());
                        status(
                            command,
                            &format!(
                                "{} peregrino registering {}",
                                Stage::Register.emoji(),
                                raw.display()
                            ),
                        )
                    })();
                    if results_sender.send((raw, result)).is_err() {
                        break;
                    }
                }
            });
        }
        drop(results_sender);
        let mut remaining = outputs.into_iter();
        let mut active = 0;
        for _ in 0..workers {
            if let Some(job) = remaining.next() {
                if verbose {
                    progress.suspend(|| {
                        eprintln!(
                            "{} peregrino: registering {}",
                            Stage::Register.emoji(),
                            job.0.display()
                        );
                    });
                }
                jobs_sender
                    .send(job)
                    .map_err(|_| anyhow::anyhow!("registration workers stopped before dispatch"))?;
                active += 1;
            }
        }
        let mut failure = None;
        while active > 0 {
            let (raw, result) = results_receiver
                .recv()
                .context("registration worker stopped unexpectedly")?;
            active -= 1;
            if let Err(error) = result {
                if failure.is_none() {
                    failure = Some(error);
                }
            } else {
                progress.set_message(
                    raw.file_name()
                        .context("raw file has no basename")?
                        .to_string_lossy()
                        .into_owned(),
                );
                progress.inc(1);
            }
            if failure.is_none()
                && let Some(job) = remaining.next()
            {
                if verbose {
                    progress.suspend(|| {
                        eprintln!(
                            "{} peregrino: registering {}",
                            Stage::Register.emoji(),
                            job.0.display()
                        );
                    });
                }
                jobs_sender
                    .send(job)
                    .map_err(|_| anyhow::anyhow!("registration workers stopped before dispatch"))?;
                active += 1;
            }
        }
        drop(jobs_sender);
        if let Some(error) = failure {
            progress.abandon_with_message("🦈 peregrino: registration stopped");
            return Err(error);
        }
        progress.finish_with_message("🦈 peregrino: registered all spectra");
        Ok(())
    })
}

#[derive(Clone, Copy)]
enum RunStatus { Complete, Partial }

fn run(config: &Config, verbose: bool, max_rows: Option<usize>) -> Result<RunStatus> {
    if let Some(max_rows) = max_rows {
        ensure!(max_rows > 0, "--topN must be positive");
    }
    if config.stages.contains(&Stage::Enrich) {
        ensure!(config.taxon_index.is_file(),
            "shared Wikidata taxon index missing: {} (run mato indexes refresh --only taxon)",
            config.taxon_index.display());
    }
    if config.stages.contains(&Stage::ChemicalEnrich) {
        ensure!(config.chemical_index.is_file(),
            "shared Wikidata chemical index missing: {} (run mato indexes refresh --only chemical)",
            config.chemical_index.display());
    }
    let mut built = HashMap::new();
    let mut raw = None;
    let mut partial_annotation = false;
    eprintln!("🐦 flock: 🦎 mato starting {} stages", config.stages.len());
    for (index, stage) in config.stages.iter().enumerate() {
        eprintln!(
            "🦎 mato: [{}/{}] {} {} — {}: {}",
            index + 1,
            config.stages.len(),
            stage.emoji(),
            stage.species(),
            stage.name(),
            stage.description()
        );
        if let Some(max_rows) = max_rows {
            if *stage != Stage::Validate {
                limited_db(&config, max_rows)?;
            }
        }
        if matches!(stage, Stage::Picoto | Stage::Register) && raw.is_none() {
            raw = Some(raw_files(&config, max_rows)?);
        }
        let species = stage.species();
        if !built.contains_key(species) {
            built.insert(species, binary(&config.flock_root, species)?);
        }
        let executable = built.get(species).context("built species missing")?;
        match stage {
            Stage::Register => register(
                &config,
                executable,
                raw.as_deref().context("raw spectra missing")?,
                verbose,
            )?,
            _ => {
                let mut command = Command::new(executable);
                let mut selected = None;
                match stage {
                    Stage::Validate => {
                        command
                            .arg("check")
                            .arg(&config.dataset_root)
                            .arg("--metadata")
                            .arg(&config.metadata)
                            .arg("--ms-dir")
                            .arg(&config.ms_dir);
                        if let Some(max_rows) = max_rows {
                            command.arg("--max-rows").arg(max_rows.to_string());
                        }
                    }
                    Stage::Ingest => {
                        command
                            .arg("ingest")
                            .arg(&config.dataset_root)
                            .arg("--doi")
                            .arg(&config.doi)
                            .arg("--metadata")
                            .arg(&config.metadata)
                            .arg("--ms-dir")
                            .arg(&config.ms_dir)
                            .arg("--config")
                            .arg(&config.path)
                            .arg("--db")
                            .arg(&config.db);
                        if let Some(max_rows) = max_rows {
                            command.arg("--max-rows").arg(max_rows.to_string());
                        }
                    }
                    Stage::Enrich => {
                        command
                            .arg("--db")
                            .arg(&config.db)
                            .arg("--index")
                            .arg(&config.taxon_index);
                        if verbose {
                            command.arg("--verbose");
                        }
                    }
                    Stage::Picoto => {
                        command
                            .arg(&config.ms_dir)
                            .arg(&config.output_dir)
                            .arg("--mode")
                            .arg("both")
                            .arg("--jobs")
                            .arg(config.picoto_jobs.to_string());
                        if max_rows.is_some() {
                            let mut manifest = tempfile::NamedTempFile::new()?;
                            for path in raw.as_deref().context("selected raw spectra missing")? {
                                let relative = path.strip_prefix(&config.ms_dir)?;
                                writeln!(manifest, "{}", relative.to_str().context("non-UTF-8 raw path")?)?;
                            }
                            manifest.flush()?;
                            selected = Some(manifest);
                        }
                    }
                    Stage::Annotate => {
                        command
                            .arg("annotate")
                            .arg("--db")
                            .arg(&config.db)
                            .arg("--library")
                            .arg(&config.library)
                            .arg("--lotus-csv")
                            .arg(&config.lotus_csv)
                            .arg("--config")
                            .arg(&config.settings);
                    }
                    Stage::ChemicalEnrich => {
                        command.arg("chemical-enrich").arg("--db").arg(&config.db)
                            .arg("--index").arg(&config.chemical_index);
                        if verbose {
                            command.arg("--verbose");
                        }
                    }
                    Stage::Register => unreachable!(),
                }
                if let Some(manifest) = &selected {
                    command.arg("--files-from").arg(manifest.path());
                }
                let description = format!("{} {} {}", stage.emoji(), stage.species(), stage.name());
                if *stage == Stage::Annotate {
                    let outcome = child_outcome(command, &description)?;
                    if outcome.code() == Some(2) {
                        partial_annotation = true;
                        eprintln!("🦎 mato: annotation files failed and were recorded; continuing to later stages for saved hits");
                    } else {
                        ensure!(outcome.success(), "{description} failed ({outcome}); stopping pipeline");
                    }
                } else {
                    status(command, &description)?;
                }
            }
        }
        if *stage == Stage::Annotate && partial_annotation {
            eprintln!("🦎 mato: {} {} partial; failed files will be retried on the next annotate run", stage.emoji(), stage.species());
        } else {
            eprintln!("🦎 mato: {} {} complete", stage.emoji(), stage.species());
        }
    }
    Ok(if partial_annotation { RunStatus::Partial } else { RunStatus::Complete })
}

fn run_reported(path: &Path, verbose: bool, top_n: Option<usize>) -> Result<()> {
    let config = Config::load(path, top_n)?;
    let started = report::start(&config);
    let outcome = run(&config, verbose, top_n);
    let status = match &outcome {
        Ok(RunStatus::Complete) => "complete",
        Ok(RunStatus::Partial) => "partial",
        Err(_) => "failed",
    };
    let saved = report::write(&config, top_n, &started, status, outcome.as_ref().err());
    match &saved {
        Ok(path) => eprintln!("🐦 flock: 🦎 mato report: {}", path.display()),
        Err(error) => eprintln!("🐦 flock: 🦎 mato could not write run report: {error:#}"),
    }
    let pipeline = match outcome {
        Ok(RunStatus::Complete) => Ok(()),
        Ok(RunStatus::Partial) => Err(anyhow::anyhow!("annotation files require reprocessing; later stages ran on saved hits (inspect annotation_runs.error)")),
        Err(error) => Err(error),
    };
    match (pipeline, saved) {
        (Ok(()), Ok(_)) => {
            eprintln!("🐦 flock: 🦎 mato pipeline complete");
            Ok(())
        },
        (Err(error), Ok(_)) => Err(error),
        (Ok(()), Err(error)) => Err(error).context("writing Mato run report"),
        (Err(error), Err(report_error)) => Err(error.context(format!("also failed to write Mato run report: {report_error:#}"))),
    }
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Action::Run { config, verbose, top_n } => run_reported(&config, verbose, top_n),
        Action::Indexes { action: IndexAction::Refresh { config, only } } =>
            refresh_indexes(config, only),
        Action::Inspect { config, top_n } => job::inspect(&Config::load(&config, top_n)?),
        Action::Reset { config, top_n, yes: _ } => job::reset(&Config::load(&config, top_n)?),
        Action::Show { config, top_n, table, limit } => {
            let config = Config::load(&config, top_n)?;
            job::require_db(&config)?;
            let peregrino = binary(&config.flock_root, "peregrino")?;
            let mut command = Command::new(peregrino);
            command
                .arg("show")
                .arg(table)
                .arg("--db")
                .arg(&config.db)
                .arg("--limit")
                .arg(limit.to_string());
            status(command, "showing database table")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn registrations_overlap_with_bounded_workers() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let ms_dir = root.join("raw");
        let output_dir = root.join("picoto");
        fs::create_dir(&ms_dir).unwrap();
        fs::create_dir(&output_dir).unwrap();
        let files: Vec<_> = ["a.mzML", "b.mzML"]
            .into_iter()
            .map(|name| {
                let raw = ms_dir.join(name);
                fs::write(&raw, []).unwrap();
                let stem = raw.file_stem().unwrap().to_str().unwrap();
                for suffix in ["_ms2.mgf", "_features.tsv", "_sirius.mgf"] {
                    fs::write(output_dir.join(format!("{stem}{suffix}")), []).unwrap();
                }
                raw
            })
            .collect();
        let binary = root.join("register-worker");
        fs::write(
            &binary,
            format!(
                r#"#!/bin/sh
while [ "$#" -gt 0 ]; do
    if [ "$1" = "--raw" ]; then shift; raw=$1; fi
    shift
done
touch "{root}/$raw.started"
i=0
while [ "$i" -lt 50 ]; do
    if [ -e "{root}/a.mzML.started" ] && [ -e "{root}/b.mzML.started" ]; then exit 0; fi
    i=$((i + 1))
    sleep 0.02
done
exit 1
"#,
                root = root.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755)).unwrap();
        let config = Config {
            path: root.join("unused.toml"),
            top_n: None,
            flock_root: root.to_path_buf(),
            stages: vec![Stage::Register],
            dataset_root: root.to_path_buf(),
            doi: String::new(),
            metadata: PathBuf::new(),
            ms_dir,
            db: root.join("unused.sqlite"),
            output_dir,
            picoto_jobs: 1,
            taxon_index: PathBuf::new(),
            peregrino_jobs: 2,
            library: PathBuf::new(),
            lotus_csv: PathBuf::new(),
            settings: PathBuf::new(),
            chemical_index: PathBuf::new(),
            indexes_config: PathBuf::new(),
        };
        register(&config, &binary, &files, false).unwrap();
    }
}
