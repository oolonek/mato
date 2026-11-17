use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

struct Fixture {
    _temporary: tempfile::TempDir,
    config: PathBuf,
    metadata: PathBuf,
    database: PathBuf,
}

impl Fixture {
    fn new(stages: &str) -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("dataset");
        let raw = root.join("peak/peak");
        let metadata_dir = root.join("metadata");
        fs::create_dir_all(&raw).unwrap();
        fs::create_dir_all(&metadata_dir).unwrap();
        fs::write(
            raw.join("sample.mzML"),
            "spectrum contents are not read by metadata tools",
        )
        .unwrap();
        let metadata = metadata_dir.join("samples.tsv");
        fs::write(&metadata, "filename\tfile_path\tdirectus_filename\ttaxon_scientific_name\tionization_mode\nsample.mzML\tdbgi_msdata/sample.mzML\tsample\tGeranium sibiricum\tpositive\n").unwrap();
        let database = temporary.path().join("flock.sqlite");
        let config = temporary.path().join("pipeline.toml");
        let flock_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
        fs::write(
            &config,
            format!(
                r#"workspace = "{}"
flock_root = "{}"
[pipeline]
stages = {stages}
[dataset]
root = "dataset"
doi = "10.25345/C55T3GD90"
metadata = "metadata/samples.tsv"
ms_dir = "peak/peak"
taxon_column = "taxon_scientific_name"
[database]
path = "flock.sqlite"
[picoto]
output_dir = "picoto"
jobs = 1
[annotation]
library = "unused.mgf"
lotus_csv = "unused.csv"
settings = "unused.toml"
[knowledge_graph]
output_dir = "toy-kg"
[indexes]
taxon_index = "taxa.sqlite"
chemical_index = "chemicals.sqlite"
"#,
                temporary.path().display(),
                flock_root.display()
            ),
        )
        .unwrap();
        Self {
            _temporary: temporary,
            config,
            metadata,
            database,
        }
    }

    fn run(&self) -> Output {
        Command::new(env!("CARGO_BIN_EXE_mato"))
            .args(["run", "--config"])
            .arg(&self.config)
            .output()
            .unwrap()
    }
}

fn reports(fixture: &Fixture) -> Vec<PathBuf> {
    let directory = fixture.database.parent().unwrap().join("reports");
    let mut files = fs::read_dir(directory).unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "md"))
        .collect::<Vec<_>>();
    files.sort();
    files
}

#[test]
fn every_completed_run_keeps_a_distinct_report_with_dataset_counts() {
    let fixture = Fixture::new("[\"validate\", \"ingest\"]");
    let first = fixture.run();
    assert!(first.status.success(), "{}", String::from_utf8_lossy(&first.stderr));
    let first_reports = reports(&fixture);
    assert_eq!(first_reports.len(), 1);
    assert!(String::from_utf8_lossy(&first.stderr).contains(first_reports[0].to_str().unwrap()));
    let saved = fs::read_to_string(&first_reports[0]).unwrap();
    assert!(saved.contains("Status: complete"), "{saved}");
    assert!(saved.contains("| Ingested sample records | 1 |"), "{saved}");
    assert!(saved.contains("| Spectrum files | 1 |"), "{saved}");
    assert!(saved.contains(fixture.database.to_str().unwrap()), "{saved}");
    assert!(saved.contains("show annotation_current_runs") && saved.contains("show annotation_runs"), "{saved}");
    assert!(saved.contains("Not available: annotation has not run"), "{saved}");

    let again = fixture.run();
    assert!(again.status.success(), "{}", String::from_utf8_lossy(&again.stderr));
    assert_eq!(reports(&fixture).len(), 2, "one report per invocation, not overwritten");
    assert_eq!(fs::read_to_string(&first_reports[0]).unwrap(), saved);
}

#[test]
fn report_write_failure_does_not_report_pipeline_success() {
    let fixture = Fixture::new("[\"validate\"]");
    fs::write(fixture.database.parent().unwrap().join("reports"), "not a directory").unwrap();
    let outcome = fixture.run();
    let error = String::from_utf8_lossy(&outcome.stderr);
    assert!(!outcome.status.success(), "{error}");
    assert!(error.contains("writing Mato run report"), "{error}");
    assert!(!error.contains("mato pipeline complete"), "{error}");
}

#[test]
fn validate_only_and_fatal_runs_report_without_creating_a_database() {
    let fixture = Fixture::new("[\"validate\"]");
    let success = fixture.run();
    assert!(success.status.success(), "{}", String::from_utf8_lossy(&success.stderr));
    let first = reports(&fixture);
    assert_eq!(first.len(), 1);
    let report = fs::read_to_string(&first[0]).unwrap();
    assert!(report.contains("Status: complete"), "{report}");
    assert!(report.contains("| Ingested sample records | not available |"), "{report}");
    assert!(!fixture.database.exists());

    fs::write(&fixture.metadata, "filename\tfile_path\tdirectus_filename\ttaxon_scientific_name\tionization_mode\nmissing.mzML\tdbgi_msdata/missing.mzML\tmissing\tGeranium sibiricum\tpositive\n").unwrap();
    let failed = fixture.run();
    assert!(!failed.status.success());
    let next = reports(&fixture);
    assert_eq!(next.len(), 2);
    let latest = next.iter().find(|path| **path != first[0]).unwrap();
    let failed_report = fs::read_to_string(latest).unwrap();
    assert!(failed_report.contains("Status: failed"), "{failed_report}");
    assert!(failed_report.contains("validate failed"), "{failed_report}");
    assert!(failed_report.contains("| Ingested sample records | not available |"), "{failed_report}");
    assert!(!fixture.database.exists());
}

#[test]
fn validates_original_columns_then_ingests_taxa_and_stops_on_a_later_bad_mapping() {
    let fixture = Fixture::new("[\"validate\", \"ingest\"]");
    let first = fixture.run();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(
        String::from_utf8_lossy(&first.stdout).contains("Imported 1 sample rows and 1 spectra")
    );
    let progress = String::from_utf8_lossy(&first.stderr);
    assert!(progress.contains("🦗 machaca"));
    assert!(progress.contains("🦈 peregrino"));
    assert!(progress.contains("🐦 flock: 🦎 mato pipeline complete"));
    let peregrino = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("peregrino/target/release/peregrino");
    let show = || {
        Command::new(&peregrino)
            .args(["show", "taxa", "--db"])
            .arg(&fixture.database)
            .output()
            .unwrap()
    };
    let before = show();
    assert!(before.status.success());
    assert!(String::from_utf8_lossy(&before.stdout).contains("Geranium sibiricum"));

    fs::write(&fixture.metadata, "filename\tfile_path\tdirectus_filename\ttaxon_scientific_name\tionization_mode\nmissing.mzML\tdbgi_msdata/sample.mzML\tsample\tNew taxon\tpositive\n").unwrap();
    let second = fixture.run();
    assert!(!second.status.success());
    assert!(String::from_utf8_lossy(&second.stderr).contains("validate failed"));
    let after = show();
    assert!(after.status.success());
    assert_eq!(
        after.stdout, before.stdout,
        "failed validation must leave the prior ingestion intact"
    );
}

#[test]
fn ingests_the_declared_header_even_when_another_taxon_header_exists() {
    let fixture = Fixture::new("[\"ingest\"]");
    fs::write(
        &fixture.metadata,
        "filename\tfile_path\tdirectus_filename\ttaxon_scientific_name\tsource_taxon\tionization_mode\nsample.mzML\tdbgi_msdata/sample.mzML\tsample\tDecoy taxon\tGeranium sibiricum\tpositive\n",
    )
    .unwrap();
    let config = fs::read_to_string(&fixture.config).unwrap();
    fs::write(
        &fixture.config,
        config.replace(
            "taxon_column = \"taxon_scientific_name\"",
            "taxon_column = \"source_taxon\"",
        ),
    )
    .unwrap();

    let output = fixture.run();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let peregrino = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("peregrino/target/release/peregrino");
    let taxa = Command::new(peregrino)
        .args(["show", "taxa", "--db"])
        .arg(&fixture.database)
        .output()
        .unwrap();
    assert!(taxa.status.success());
    let actual = String::from_utf8_lossy(&taxa.stdout);
    assert!(actual.contains("Geranium sibiricum"), "{actual}");
    assert!(!actual.contains("Decoy taxon"), "{actual}");
}

#[test]
fn rejects_reordered_stages_without_touching_database() {
    let fixture = Fixture::new("[\"ingest\", \"validate\"]");
    let output = fixture.run();
    assert!(!output.status.success());
    assert!(!fixture.database.exists());
}

#[test]
fn enrich_requires_a_shared_taxon_index_before_any_stage_runs() {
    let fixture = Fixture::new("[\"ingest\", \"enrich\"]");
    let output = fixture.run();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("shared Wikidata taxon index missing"));
    assert!(!fixture.database.exists());
}

#[test]
fn refresh_one_or_both_indexes_from_one_config_and_match_taxon() {
    let fixture = Fixture::new("[\"ingest\", \"enrich\"]");
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/sparql", listener.local_addr().unwrap());
    let worker = std::thread::spawn(move || {
        let count = r#"{"results":{"bindings":[
            {"property":{"value":"http://www.wikidata.org/prop/direct/P225"},"count":{"value":"1"}},
            {"property":{"value":"http://www.wikidata.org/prop/direct/P3151"},"count":{"value":"1"}}]}}"#;
        let name = "item,value\nhttp://www.wikidata.org/entity/Q888,Geranium sibiricum\n";
        let inat = "item,value\nhttp://www.wikidata.org/entity/Q888,42\n";
        for body in [count, name, inat, count, name, inat] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() { break; }
            }
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            stream.flush().unwrap();
        }
    });
    let shared = fs::read_to_string(&fixture.config).unwrap();
    fs::write(&fixture.config, format!("{shared}taxon_endpoint = \"{endpoint}\"\n")).unwrap();
    let refresh = Command::new(env!("CARGO_BIN_EXE_mato"))
        .args(["indexes", "refresh", "--only", "taxon", "--config"])
        .arg(&fixture.config).output().unwrap();
    assert!(refresh.status.success(), "{}", String::from_utf8_lossy(&refresh.stderr));
    assert!(fixture._temporary.path().join("taxa.sqlite").is_file());
    assert!(!fixture._temporary.path().join("chemicals.sqlite").exists());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let chemical_endpoint = format!("http://{}/sparql", listener.local_addr().unwrap());
    let chemical_worker = std::thread::spawn(move || {
        let count = r#"{"results":{"bindings":[{"count":{"value":"1"}}]}}"#;
        let csv = "item,key,cid,name\nhttp://www.wikidata.org/entity/Q77,JBFOLLJCGUCDQP-UHFFFAOYSA-N,42,compound\n";
        for body in [count, csv] {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() { break; }
            }
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            stream.flush().unwrap();
        }
    });
    fs::write(&fixture.config,
        format!("{shared}taxon_endpoint = \"{endpoint}\"\nchemical_endpoint = \"{chemical_endpoint}\"\n")
    ).unwrap();
    let both = Command::new(env!("CARGO_BIN_EXE_mato"))
        .args(["indexes", "refresh", "--config"])
        .arg(&fixture.config).output().unwrap();
    assert!(both.status.success(), "{}", String::from_utf8_lossy(&both.stderr));
    worker.join().unwrap();
    chemical_worker.join().unwrap();
    let chemicals = rusqlite::Connection::open(fixture._temporary.path().join("chemicals.sqlite")).unwrap();
    let qid: String = chemicals.query_row(
        "SELECT wikidata_qid FROM wikidata_chemical_index WHERE inchikey='JBFOLLJCGUCDQP-UHFFFAOYSA-N'",
        [], |row| row.get(0)
    ).unwrap();
    assert_eq!(qid, "Q77");

    let output = fixture.run();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let taxon = Command::new(env!("CARGO_BIN_EXE_mato"))
        .args(["show", "taxon_resolutions", "--config"])
        .arg(&fixture.config).output().unwrap();
    assert!(taxon.status.success());
    assert!(String::from_utf8_lossy(&taxon.stdout).contains("Q888"));
    let identifiers = Command::new(env!("CARGO_BIN_EXE_mato"))
        .args(["show", "taxon_resolution_ids", "--config"])
        .arg(&fixture.config).output().unwrap();
    assert!(identifiers.status.success());
    assert!(String::from_utf8_lossy(&identifiers.stdout).contains("P3151\t42"));
}

#[test]
fn rejects_zero_registration_workers_before_touching_database() {
    let fixture = Fixture::new("[\"validate\"]");
    let base = fs::read_to_string(&fixture.config).unwrap();
    fs::write(&fixture.config, format!("{base}\n[peregrino]\njobs = 0\n")).unwrap();
    let output = fixture.run();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("peregrino.jobs"));
    assert!(!fixture.database.exists());
}

#[test]
fn rejects_missing_taxon_column_before_touching_database() {
    let fixture = Fixture::new("[\"ingest\"]");
    let config = fs::read_to_string(&fixture.config).unwrap();
    fs::write(
        &fixture.config,
        config.replace("taxon_column = \"taxon_scientific_name\"\n", ""),
    )
    .unwrap();
    let output = fixture.run();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("taxon_column"));
    assert!(!fixture.database.exists());
}

#[test]
fn rejects_empty_taxon_column_before_touching_database() {
    let fixture = Fixture::new("[\"ingest\"]");
    let config = fs::read_to_string(&fixture.config).unwrap();
    fs::write(
        &fixture.config,
        config.replace(
            "taxon_column = \"taxon_scientific_name\"",
            "taxon_column = \" \"",
        ),
    )
    .unwrap();
    let output = fixture.run();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("dataset.taxon_column"));
    assert!(!fixture.database.exists());
}

#[test]
fn rejects_nonexistent_taxon_column_before_touching_database() {
    let fixture = Fixture::new("[\"ingest\"]");
    let config = fs::read_to_string(&fixture.config).unwrap();
    fs::write(
        &fixture.config,
        config.replace(
            "taxon_column = \"taxon_scientific_name\"",
            "taxon_column = \"wrong_header\"",
        ),
    )
    .unwrap();
    let output = fixture.run();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("wrong_header"));
    assert!(!fixture.database.exists());
}

#[test]
fn top_n_uses_only_selected_rows_and_independent_jobs_without_touching_the_default_job() {
    let fixture = Fixture::new("[\"validate\"]");
    let config = fs::read_to_string(&fixture.config).unwrap();
    fs::write(
        &fixture.config,
        config
            .replace(
                "stages = [\"validate\"]",
                "stages = [\"validate\"]\ntop_n_stages = [\"validate\", \"ingest\"]",
            )
            .replace("[database]\n", "[database]\ntop_n_root = \"top-n\"\n"),
    ).unwrap();
    let raw = fixture.metadata.parent().unwrap().parent().unwrap().join("peak/peak");
    fs::write(raw.join("later.mzML"), "not read during ingestion").unwrap();
    fs::write(
        &fixture.metadata,
        "filename\tfile_path\tdirectus_filename\ttaxon_scientific_name\tionization_mode\n\
         sample.mzML\tdbgi_msdata/sample.mzML\tsample\tGeranium sibiricum\tpositive\n\
         later.mzML\tdbgi_msdata/later.mzML\tlater\tLater species\tnegative\n",
    ).unwrap();
    let parent = fixture.database.parent().unwrap();
    for (n, expected) in [(1, vec!["sample.mzML"]), (2, vec!["later.mzML", "sample.mzML"])] {
        let selected = Command::new(env!("CARGO_BIN_EXE_mato"))
            .arg("run")
            .arg("--config")
            .arg(&fixture.config)
            .args(["--topN", &n.to_string()])
            .output().unwrap();
        assert!(selected.status.success(), "{}", String::from_utf8_lossy(&selected.stderr));
        let database = parent.join(format!("top-n/{n}/flock.sqlite"));
        let db = rusqlite::Connection::open(&database).unwrap();
        let files: Vec<String> = db.prepare("SELECT filename FROM spectrum_files ORDER BY filename")
            .unwrap().query_map([], |row| row.get(0)).unwrap()
            .collect::<rusqlite::Result<_>>().unwrap();
        assert_eq!(files, expected);
    }
    let full = fixture.run();
    assert!(full.status.success(), "{}", String::from_utf8_lossy(&full.stderr));
    assert!(!fixture.database.exists(), "default validate-only job must not ingest");
    let first = rusqlite::Connection::open(parent.join("top-n/1/flock.sqlite")).unwrap();
    let rows: i64 = first.query_row("SELECT count(*) FROM sample_rows", [], |row| row.get(0)).unwrap();
    assert_eq!(rows, 1, "a distinct top-N job must not expand the first one");
}

#[test]
fn filters_and_sorts_metadata_before_validate_ingest_and_top_n() {
    let fixture = Fixture::new("[\"validate\"]");
    let config = fs::read_to_string(&fixture.config).unwrap()
        .replace("stages = [\"validate\"]", "stages = [\"validate\"]\ntop_n_stages = [\"validate\", \"ingest\"]")
        .replace("[database]\n", "[database]\ntop_n_root = \"top-n\"\n");
    fs::write(&fixture.config, format!("{config}\n[selection]\nrequire_nonempty = [\"observation_subject\", \"collector_full_name\"]\n[[selection.sort]]\ncolumn = \"filename\"\ndescending = true\n")).unwrap();
    let raw = fixture.metadata.parent().unwrap().parent().unwrap().join("peak/peak");
    for name in ["b.mzML", "c.mzML", "d.mzML"] {
        fs::write(raw.join(name), format!("{name} spectral bytes")).unwrap();
    }
    let original = "filename\tfile_path\tdirectus_filename\ttaxon_scientific_name\tionization_mode\tobservation_subject\tcollector_full_name\n\
        sample.mzML\tdbgi_msdata/sample.mzML\tsample\tGeranium sibiricum\tpositive\t\tAda\n\
        b.mzML\tdbgi_msdata/b.mzML\tb\tGeranium sibiricum\tpositive\tLeaf\tAda\n\
        c.mzML\tdbgi_msdata/c.mzML\tc\tGeranium sibiricum\tpositive\tStems\tBob\n\
        d.mzML\tdbgi_msdata/d.mzML\td\tGeranium sibiricum\tpositive\tFlower\t\n";
    fs::write(&fixture.metadata, original).unwrap();
    let run = || Command::new(env!("CARGO_BIN_EXE_mato"))
        .args(["run", "--config"]).arg(&fixture.config).args(["--topN", "1"]).output().unwrap();
    let result = run();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let db_path = fixture.database.parent().unwrap().join("top-n/1/flock.sqlite");
    let db = rusqlite::Connection::open(&db_path).unwrap();
    let selected: (String, String) = db.query_row(
        "SELECT f.filename, s.observation_subject FROM spectrum_files f JOIN sample_rows s USING(dataset_id,row_number)",
        [], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(selected, ("c.mzML".into(), "Stems".into()));
    let snapshot: String = db.query_row("SELECT metadata_path FROM datasets", [], |row| row.get(0)).unwrap();
    assert!(snapshot.contains("/.mato-selections/"), "{snapshot}");
    assert_eq!(fs::read_to_string(&fixture.metadata).unwrap(), original);
    let again = run();
    assert!(again.status.success(), "{}", String::from_utf8_lossy(&again.stderr));
    let changed = fs::read_to_string(&fixture.config).unwrap().replace("descending = true", "descending = false");
    fs::write(&fixture.config, changed).unwrap();
    let rejected = run();
    assert!(!rejected.status.success());
    assert!(String::from_utf8_lossy(&rejected.stderr).contains("isolated database"),
        "{}", String::from_utf8_lossy(&rejected.stderr));
    let unchanged: String = db.query_row("SELECT filename FROM spectrum_files", [], |row| row.get(0)).unwrap();
    assert_eq!(unchanged, "c.mzML");
}
