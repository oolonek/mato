use std::fs;
use std::path::Path;
use std::process::Command;

use rusqlite::{Connection, params};

#[test]
fn chemical_stage_uses_configured_shared_index_instead_of_reusing_stale_job_lookup() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let dataset = root.join("dataset");
    fs::create_dir(&dataset).unwrap();
    fs::write(dataset.join("metadata.tsv"), "taxon\nPlant\n").unwrap();
    fs::create_dir(dataset.join("raw")).unwrap();
    let key = "JBFOLLJCGUCDQP-UHFFFAOYSA-N";
    let db_path = root.join("job.sqlite");
    let db = Connection::open(&db_path).unwrap();
    db.execute_batch(
        "CREATE TABLE processed_spectra(dataset_id INTEGER,filename TEXT,ms2_sha256 TEXT);
         CREATE TABLE annotation_runs(id INTEGER,dataset_id INTEGER,filename TEXT,ms2_sha256 TEXT,status TEXT);
         CREATE TABLE annotation_hits(run_id INTEGER,feature_id TEXT,rank INTEGER,library_inchikey TEXT);
         CREATE VIEW annotation_current_runs AS SELECT r.id,r.status FROM annotation_runs r
           JOIN processed_spectra p USING(dataset_id,filename) WHERE r.ms2_sha256=p.ms2_sha256;
         CREATE TABLE chemical_lookups(inchikey TEXT PRIMARY KEY,status TEXT,checked_at TEXT);
         INSERT INTO processed_spectra VALUES(1,'raw.mzML','sha');
         INSERT INTO annotation_runs VALUES(1,1,'raw.mzML','sha','hits');",
    ).unwrap();
    db.execute("INSERT INTO annotation_hits VALUES(1,'f',1,?1)", [key])
        .unwrap();
    db.execute(
        "INSERT INTO chemical_lookups VALUES(?1,'matched','yesterday')",
        [key],
    )
    .unwrap();
    let shared = root.join("shared.sqlite");
    let index = Connection::open(&shared).unwrap();
    index.execute_batch(
        "CREATE TABLE wikidata_chemical_snapshot(id INTEGER PRIMARY KEY,generation INTEGER,fetched_at TEXT,endpoint TEXT,pairs INTEGER);
         CREATE TABLE wikidata_chemical_index(inchikey TEXT,wikidata_qid TEXT,name TEXT,PRIMARY KEY(inchikey,wikidata_qid));
         CREATE TABLE wikidata_chemical_cids(inchikey TEXT,wikidata_qid TEXT,pubchem_cid TEXT,PRIMARY KEY(inchikey,wikidata_qid,pubchem_cid));
         INSERT INTO wikidata_chemical_snapshot VALUES(1,1,'today','fixture',1);",
    ).unwrap();
    index
        .execute(
            "INSERT INTO wikidata_chemical_index VALUES(?1,'Q77','compound')",
            [key],
        )
        .unwrap();
    index
        .execute(
            "INSERT INTO wikidata_chemical_cids VALUES(?1,'Q77','42')",
            [key],
        )
        .unwrap();
    drop(index);

    let config = root.join("job.toml");
    fs::write(root.join("wikidata.toml"),
        "[indexes]\ntaxon_index = \"taxa.sqlite\"\nchemical_index = \"shared.sqlite\"\n"
    ).unwrap();
    let flock_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    fs::write(&config, format!(
        "flock_root = {:?}\nwikidata_config = \"wikidata.toml\"\n[pipeline]\nstages = [\"chemical_enrich\"]\n[dataset]\nroot = {:?}\ndoi = \"doi\"\nmetadata = \"metadata.tsv\"\nms_dir = \"raw\"\ntaxon_column = \"taxon\"\n[database]\npath = {:?}\n[picoto]\noutput_dir = {:?}\njobs = 1\n[annotation]\nlibrary = \"unused.mgf\"\nlotus_csv = \"unused.csv\"\nsettings = \"unused.toml\"\n",
        flock_root.display().to_string(), dataset.display().to_string(), db_path.display().to_string(), root.join("picoto").display().to_string()
    )).unwrap();
    let outcome = Command::new(env!("CARGO_BIN_EXE_mato"))
        .args(["run", "--config"])
        .arg(&config)
        .output()
        .unwrap();
    assert!(
        outcome.status.success(),
        "{}",
        String::from_utf8_lossy(&outcome.stderr)
    );
    let chemical: (String, String) = db.query_row(
        "SELECT e.wikidata_qid,p.pubchem_cid FROM chemical_entities e JOIN chemical_pubchem_ids p USING(inchikey,wikidata_qid) WHERE e.inchikey=?1",
        params![key],
        |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(chemical, ("Q77".into(), "42".into()));
    assert!(shared.is_file());
}
