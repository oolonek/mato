use std::fs;
use std::path::Path;
use std::process::Command;

#[test]
fn bootstrap_refuses_an_incomplete_existing_wikidata_index_without_destroying_it() {
    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("workspace");
    let index = workspace.join("mato/wikidata/taxa.sqlite");
    fs::create_dir_all(index.parent().unwrap()).unwrap();
    fs::write(&index, b"not a completed SQLite snapshot").unwrap();
    let config = temporary.path().join("toy.toml");
    let flock = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    fs::write(
        &config,
        format!(
            "flock_root = \"{}\"\nworkspace = \"{}\"\n[pipeline]\nstages = [\"validate\", \"ingest\", \"enrich\", \"picoto\", \"register\", \"annotate\", \"chemical_enrich\"]\n[dataset]\nroot = \"sapayoa/zenodo-10018590\"\ndoi = \"10.5281/zenodo.10018264\"\nmetadata = \"metadata/metadata.tsv\"\nms_dir = \"msdata/raw\"\ntaxon_column = \"source_taxon\"\n[database]\npath = \"mato/toy.sqlite\"\n[picoto]\noutput_dir = \"mato/toy-picoto\"\njobs = 2\n[indexes]\ntaxon_index = \"mato/wikidata/taxa.sqlite\"\nchemical_index = \"mato/wikidata/chemicals.sqlite\"\n[knowledge_graph]\noutput_dir = \"mato/toy-kg\"\n[annotation]\nlibrary = \"hocofaisan/references/isdb_lotus_pos_energySum.mgf\"\nlotus_csv = \"hocofaisan/references/230106_frozen_metadata.csv\"\nsettings = \"{}\"\n",
            flock.display(),
            workspace.display(),
            flock.join("hocofaisan/config/example.toml").display()
        ),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_mato"))
        .args(["bootstrap", "--config"])
        .arg(&config)
        .output()
        .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("taxon index") && stderr.contains("incomplete"), "{stderr}");
    assert_eq!(fs::read(&index).unwrap(), b"not a completed SQLite snapshot");
    assert!(!workspace.join("mato/toy.sqlite").exists());
}
