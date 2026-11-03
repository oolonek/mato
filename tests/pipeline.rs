use std::fs;
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
                r#"flock_root = "{}"
[pipeline]
stages = {stages}
[dataset]
root = "{}"
doi = "10.25345/C55T3GD90"
metadata = "metadata/samples.tsv"
ms_dir = "peak/peak"
taxon_column = "taxon_scientific_name"
[database]
path = "{}"
[picoto]
output_dir = "{}"
jobs = 1
[annotation]
library = "unused.mgf"
lotus_csv = "unused.csv"
settings = "unused.toml"
"#,
                flock_root.display(),
                root.display(),
                database.display(),
                temporary.path().join("picoto").display()
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
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("pipeline stages must be unique and in this order")
    );
    assert!(!fixture.database.exists());
}

#[test]
fn rejects_out_of_range_cunaguaro_jobs_before_touching_database() {
    let fixture = Fixture::new("[\"validate\"]");
    let base = fs::read_to_string(&fixture.config).unwrap();
    for jobs in [0, 4] {
        fs::write(
            &fixture.config,
            format!("{base}\n[cunaguaro]\njobs = {jobs}\n"),
        )
        .unwrap();
        let output = fixture.run();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("cunaguaro.jobs"));
        assert!(!fixture.database.exists());
    }
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
