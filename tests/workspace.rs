use std::fs;
use std::path::Path;
use std::process::Command;

#[test]
fn one_workspace_root_places_ingested_data_and_job_outputs_without_other_absolute_paths() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp.path().join("workspace");
    let dataset = workspace.join("sapayoa/zenodo-toy");
    let raw = dataset.join("msdata/raw");
    fs::create_dir_all(&raw).unwrap();
    fs::write(raw.join("run.mzXML"), "raw spectrum").unwrap();
    fs::create_dir_all(dataset.join("metadata")).unwrap();
    fs::write(
        dataset.join("metadata/metadata.tsv"),
        "sample_id\tsource_taxon\tsample_filename_pos\nA\tSalvia officinalis\trun.mzXML\n",
    )
    .unwrap();
    let config = temp.path().join("toy.toml");
    let flock = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    fs::write(
        &config,
        format!(
            "flock_root = \"{}\"\nworkspace = \"{}\"\n[pipeline]\nstages = [\"validate\", \"ingest\"]\n[dataset]\nroot = \"sapayoa/zenodo-toy\"\ndoi = \"10.5281/zenodo.10018264\"\nmetadata = \"metadata/metadata.tsv\"\nms_dir = \"msdata/raw\"\ntaxon_column = \"source_taxon\"\n[database]\npath = \"jobs/toy.sqlite\"\n[picoto]\noutput_dir = \"jobs/toy-picoto\"\njobs = 1\n[indexes]\ntaxon_index = \"shared/taxa.sqlite\"\nchemical_index = \"shared/chemicals.sqlite\"\n[knowledge_graph]\noutput_dir = \"jobs/toy-kg\"\n[annotation]\nlibrary = \"references/isdb.mgf\"\nlotus_csv = \"references/lotus.csv\"\nsettings = \"{}\"\n",
            flock.display(),
            workspace.display(),
            flock.join("hocofaisan/config/example.toml").display()
        ),
    )
    .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_mato"))
        .args(["run", "--config"])
        .arg(&config)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let db = workspace.join("jobs/toy.sqlite");
    assert!(db.is_file(), "ingest did not create {}", db.display());
    let preview = Command::new(env!("CARGO_BIN_EXE_mato"))
        .args(["show", "datasets", "--config"])
        .arg(&config)
        .output()
        .unwrap();
    assert!(preview.status.success(), "{}", String::from_utf8_lossy(&preview.stderr));
    let output = String::from_utf8(preview.stdout).unwrap();
    assert!(output.contains("10.5281/zenodo.10018264"), "{output}");
    assert!(output.contains(dataset.to_str().unwrap()), "{output}");
}
