use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Job {
    _temporary: tempfile::TempDir,
    config: PathBuf,
    database: PathBuf,
    output: PathBuf,
    dataset: PathBuf,
}

impl Job {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let dataset = temporary.path().join("dataset");
        fs::create_dir_all(dataset.join("raw")).unwrap();
        fs::write(dataset.join("metadata.tsv"), "taxon\nExample species\n").unwrap();
        let database = temporary.path().join("job.sqlite");
        let output = temporary.path().join("picoto");
        let config = temporary.path().join("job.toml");
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        fs::write(
            &config,
            format!(
                "flock_root = {:?}\n[pipeline]\nstages = [\"validate\", \"ingest\"]\n[dataset]\nroot = \"dataset\"\ndoi = \"10.1234/example\"\nmetadata = \"metadata.tsv\"\nms_dir = \"raw\"\ntaxon_column = \"taxon\"\n[database]\npath = \"job.sqlite\"\n[picoto]\noutput_dir = \"picoto\"\njobs = 1\n[annotation]\nlibrary = \"unused.mgf\"\nlotus_csv = \"unused.csv\"\nsettings = \"unused.toml\"\n",
                root.display().to_string()
            ),
        )
        .unwrap();
        Self {
            _temporary: temporary,
            config,
            database,
            output,
            dataset,
        }
    }

    fn cli(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_mato"))
            .args(args)
            .arg("--config")
            .arg(&self.config)
            .output()
            .unwrap()
    }
}

#[test]
fn inspect_identifies_missing_job_db_and_show_explains_how_to_create_it() {
    let job = Job::new();
    let inspect = job.cli(&["inspect"]);
    assert!(
        inspect.status.success(),
        "{}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let report = String::from_utf8_lossy(&inspect.stdout);
    assert!(report.contains(&format!("database: {} (missing)", job.database.display())));
    assert!(report.contains(&format!("Picoto: {} (missing)", job.output.display())));
    assert!(report.contains("DOI: 10.1234/example"));
    assert!(report.contains("stages: validate, ingest"));

    let show = job.cli(&["show", "annotation_current_runs"]);
    assert!(!show.status.success());
    let error = String::from_utf8_lossy(&show.stderr);
    assert!(error.contains("database does not exist"), "{error}");
    assert!(error.contains("mato run --config"), "{error}");
    assert!(
        !job.database.exists(),
        "read-only commands must not create a database"
    );
}

#[test]
fn inspect_reports_ingested_rows_before_annotation_tables_exist() {
    let job = Job::new();
    let db = rusqlite::Connection::open(&job.database).unwrap();
    db.execute_batch(
        "CREATE TABLE datasets (id INTEGER, doi TEXT);
         INSERT INTO datasets VALUES (1, '10.1234/example');
         CREATE TABLE sample_rows (dataset_id INTEGER);
         INSERT INTO sample_rows VALUES (1);",
    )
    .unwrap();
    drop(db);
    let inspect = job.cli(&["inspect"]);
    assert!(
        inspect.status.success(),
        "{}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let report = String::from_utf8_lossy(&inspect.stdout);
    assert!(
        report.contains("stored dataset: 1 10.1234/example"),
        "{report}"
    );
    assert!(report.contains("sample_rows: 1"), "{report}");
    assert!(
        report.contains("annotation_runs: not yet created"),
        "{report}"
    );
}

#[test]
fn inspect_distinguishes_historical_annotation_attempts_from_current_files() {
    let job = Job::new();
    let db = rusqlite::Connection::open(&job.database).unwrap();
    db.execute_batch(
        "CREATE TABLE sample_rows (dataset_id INTEGER);
         INSERT INTO sample_rows VALUES(1),(1);
         CREATE TABLE spectrum_files (dataset_id INTEGER,filename TEXT);
         INSERT INTO spectrum_files VALUES(1,'a.mzML'),(1,'b.mzML');
         CREATE TABLE annotation_runs(id INTEGER,filename TEXT,status TEXT);
         INSERT INTO annotation_runs VALUES
            (1,'a.mzML','error'),(2,'a.mzML','error'),(3,'b.mzML','hits');
         CREATE VIEW annotation_current_runs AS
            SELECT r.id,r.filename,r.status FROM annotation_runs r
            WHERE r.id=(SELECT MAX(id) FROM annotation_runs WHERE filename=r.filename);"
    ).unwrap();
    let inspect = job.cli(&["inspect"]);
    assert!(inspect.status.success(), "{}", String::from_utf8_lossy(&inspect.stderr));
    let report = String::from_utf8_lossy(&inspect.stdout);
    assert!(report.contains("sample_rows: 2"), "{report}");
    assert!(report.contains("spectrum_files: 2"), "{report}");
    assert!(report.contains("annotation_runs: 3 historical attempts"), "{report}");
    assert!(report.contains("annotation_current_runs: 2 current files (1 errors)"), "{report}");
}

#[test]
fn reset_requires_confirmation_and_removes_only_the_job_database_and_picoto_outputs() {
    let job = Job::new();
    let db = rusqlite::Connection::open(&job.database).unwrap();
    db.execute_batch("CREATE TABLE job_marker (value INTEGER); INSERT INTO job_marker VALUES (7)")
        .unwrap();
    drop(db);
    for suffix in ["-wal", "-shm", "-journal"] {
        fs::write(
            format!("{}{suffix}", job.database.display()),
            b"old sidecar",
        )
        .unwrap();
    }
    fs::create_dir_all(&job.output).unwrap();
    fs::write(job.output.join("feature.tsv"), b"old output").unwrap();
    let neighboring_file = job.database.with_file_name("unrelated.sqlite");
    fs::write(&neighboring_file, b"keep").unwrap();
    let saved_report = job.database.parent().unwrap().join("reports").join("previous.md");
    fs::create_dir_all(saved_report.parent().unwrap()).unwrap();
    fs::write(&saved_report, b"previous run report").unwrap();

    let denied = job.cli(&["reset"]);
    assert!(!denied.status.success());
    assert!(job.database.exists() && job.output.exists());
    let reset = job.cli(&["reset", "--yes"]);
    assert!(
        reset.status.success(),
        "{}",
        String::from_utf8_lossy(&reset.stderr)
    );
    assert!(!job.database.exists());
    assert!(!job.output.exists());
    for suffix in ["-wal", "-shm", "-journal"] {
        assert!(!PathBuf::from(format!("{}{suffix}", job.database.display())).exists());
    }
    assert!(job.dataset.join("metadata.tsv").exists());
    assert!(neighboring_file.exists());
    assert!(saved_report.exists(), "reset should preserve prior run reports");
}

#[test]
fn reset_refuses_an_in_use_database_and_preserves_outputs() {
    let job = Job::new();
    let writer = rusqlite::Connection::open(&job.database).unwrap();
    writer
        .execute_batch("CREATE TABLE marker (id INTEGER); BEGIN EXCLUSIVE;")
        .unwrap();
    fs::create_dir_all(&job.output).unwrap();
    fs::write(job.output.join("feature.tsv"), b"keep").unwrap();

    let reset = job.cli(&["reset", "--yes"]);
    assert!(!reset.status.success());
    assert!(String::from_utf8_lossy(&reset.stderr).contains("database is in use"));
    assert!(job.database.exists());
    assert!(job.output.join("feature.tsv").exists());
    writer.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn reset_rejects_a_picoto_path_containing_the_dataset() {
    let job = Job::new();
    let config = fs::read_to_string(&job.config).unwrap();
    fs::write(
        &job.config,
        config.replace("output_dir = \"picoto\"", "output_dir = \".\""),
    )
    .unwrap();
    fs::write(&job.database, b"keep database").unwrap();
    let reset = job.cli(&["reset", "--yes"]);
    assert!(!reset.status.success());
    assert!(String::from_utf8_lossy(&reset.stderr).contains("unsafe Picoto output directory"));
    assert!(job.database.exists());
    assert!(job.dataset.join("metadata.tsv").exists());
}

#[test]
fn top_n_inspect_show_and_reset_target_only_the_selected_job() {
    let job = Job::new();
    let config = fs::read_to_string(&job.config).unwrap();
    fs::write(
        &job.config,
        config
            .replace(
                "stages = [\"validate\", \"ingest\"]",
                "stages = [\"validate\", \"ingest\"]\ntop_n_stages = [\"validate\"]",
            )
            .replace("[database]\n", "[database]\ntop_n_root = \"top-n\"\n"),
    ).unwrap();
    let base = job.database.parent().unwrap().join("top-n");
    let limited = base.join("250/flock.sqlite");
    let other = base.join("200/flock.sqlite");
    fs::create_dir_all(limited.parent().unwrap()).unwrap();
    fs::create_dir_all(other.parent().unwrap()).unwrap();
    fs::write(&job.database, b"production data").unwrap();
    fs::write(&other, b"other top-N job").unwrap();
    fs::write(base.join("flock.sqlite"), b"old top-N job").unwrap();
    let db = rusqlite::Connection::open(&limited).unwrap();
    db.execute_batch("CREATE TABLE marker (id INTEGER)").unwrap();
    drop(db);
    let picoto = base.join("250/picoto");
    fs::create_dir_all(&picoto).unwrap();
    fs::write(picoto.join("feature.tsv"), b"generated").unwrap();

    let inspect = job.cli(&["inspect", "--topN", "250"]);
    assert!(inspect.status.success(), "{}", String::from_utf8_lossy(&inspect.stderr));
    let status = String::from_utf8_lossy(&inspect.stdout);
    assert!(status.contains("stages: validate"), "{status}");
    assert!(!status.contains("stages: validate, ingest"), "{status}");
    assert!(status.contains(&limited.display().to_string()), "{status}");
    let show = job.cli(&["show", "annotation_current_runs", "--topN", "300"]);
    assert!(!show.status.success());
    assert!(String::from_utf8_lossy(&show.stderr).contains(" --topN 300"));
    let reset = job.cli(&["reset", "--topN", "250", "--yes"]);
    assert!(reset.status.success(), "{}", String::from_utf8_lossy(&reset.stderr));
    assert!(!limited.exists() && !picoto.exists());
    assert!(job.database.exists() && other.exists() && base.join("flock.sqlite").exists());
}

#[test]
fn top_n_without_isolation_settings_or_positive_count_cannot_touch_the_default_job() {
    let job = Job::new();
    let absent = job.cli(&["inspect", "--topN", "1"]);
    assert!(!absent.status.success());
    assert!(String::from_utf8_lossy(&absent.stderr).contains("top_n_root"));
    let zero = job.cli(&["reset", "--topN", "0", "--yes"]);
    assert!(!zero.status.success());
    assert!(String::from_utf8_lossy(&zero.stderr).contains("--topN must be greater than zero"));
    assert!(!job.database.exists());
}
