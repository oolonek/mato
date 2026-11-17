use std::fs;
use std::path::Path;
use std::process::Command;

#[test]
fn run_refuses_an_incomplete_existing_wikidata_index_without_destroying_it() {
    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("workspace");
    let index = workspace.join("mato/wikidata/taxa.sqlite");
    fs::create_dir_all(index.parent().unwrap()).unwrap();
    fs::write(&index, b"not a completed SQLite snapshot").unwrap();
    let dataset = workspace.join("sapayoa/zenodo-10018590");
    fs::create_dir_all(dataset.join("msdata/raw")).unwrap();
    fs::create_dir_all(dataset.join("metadata")).unwrap();
    fs::write(dataset.join("metadata/metadata.tsv"), b"sample_id\tsource_taxon\n").unwrap();
    let config = temporary.path().join("toy.toml");
    let flock = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    fs::write(
        &config,
        format!(
            r#"flock_root = "{flock}"
workspace = "{workspace}"
[pipeline]
stages = ["enrich"]
[dataset]
root = "sapayoa/zenodo-10018590"
doi = "10.5281/zenodo.10018264"
metadata = "metadata/metadata.tsv"
ms_dir = "msdata/raw"
taxon_column = "source_taxon"
[database]
path = "mato/toy.sqlite"
[picoto]
output_dir = "mato/toy-picoto"
jobs = 2
[indexes]
taxon_index = "mato/wikidata/taxa.sqlite"
chemical_index = "mato/wikidata/chemicals.sqlite"
create_missing = true
[knowledge_graph]
output_dir = "mato/toy-kg"
[annotation]
library = "unused.mgf"
lotus_csv = "unused.csv"
settings = "unused.toml"
"#,
            flock = flock.display(),
            workspace = workspace.display(),
        ),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_mato"))
        .args(["run", "--config"])
        .arg(&config)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("taxon") && stderr.contains("snapshot"), "{stderr}");
    assert_eq!(fs::read(&index).unwrap(), b"not a completed SQLite snapshot");
    assert!(!workspace.join("mato/toy.sqlite").exists());
}
