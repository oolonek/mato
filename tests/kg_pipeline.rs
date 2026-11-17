use std::fs;
use std::path::Path;
use std::process::Command;

use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn partial_fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    let dataset = root.join("dataset");
    let raw = dataset.join("raw");
    fs::create_dir_all(&raw).unwrap();
    let metadata = dataset.join("metadata.tsv");
    fs::write(&metadata, "sample_id\tsource_taxon\nA\tSalvia officinalis\nB\tSalvia officinalis\n").unwrap();
    fs::write(raw.join("run.mzML"), b"raw spectral bytes").unwrap();
    fs::write(raw.join("missing.mzML"), b"unprocessed raw spectral bytes").unwrap();
    let audit = root.join("run_features.tsv");
    fs::write(&audit, "feature_id\tmz\trt_seconds\tapex_intensity\tapex_to_baseline\tms1_scans\tarea\tisotope_peaks\tquality_score\tstatus\treason\tms2_scan_ids\tms2_quality\tcollision_energy\n1\t123.45\t15.5\t100\t3\t3\t100\t0\t5\taccepted\t\t1\t1\t\n").unwrap();
    let mgf = root.join("run_ms2.mgf");
    let mgf_bytes = b"BEGIN IONS\nFEATURE_ID=1\nTITLE=run.mzML:feature_1:MS2\nMSLEVEL=2\nFILENAME=run.mzML\nPEPMASS=123.45000000\nCHARGE=1+\nRTINSECONDS=15.5000\n100 20\nEND IONS\n";
    fs::write(&mgf, mgf_bytes).unwrap();
    let matcher = root.join("matcher.toml");
    fs::write(&matcher, "[search]\nmetric = 'ModifiedLinearCosine'\n").unwrap();
    let db_path = root.join("top-n/2/flock.sqlite");
    fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    let db = Connection::open(&db_path).unwrap();
    db.execute_batch("CREATE TABLE datasets (id INTEGER PRIMARY KEY,doi TEXT,root TEXT,metadata_path TEXT);
        CREATE TABLE taxa(id INTEGER PRIMARY KEY,name TEXT);
        CREATE TABLE taxon_resolutions(taxon_id INTEGER,qid TEXT);
        CREATE TABLE sample_rows(dataset_id INTEGER,row_number INTEGER,sample_id TEXT,taxon_id INTEGER,attributes_json TEXT);
        CREATE TABLE spectrum_files(dataset_id INTEGER,filename TEXT,row_number INTEGER,relative_path TEXT);
        CREATE TABLE processed_spectra(dataset_id INTEGER,filename TEXT,ms2_path TEXT,ms2_sha256 TEXT,features_path TEXT,paired_path TEXT);
        CREATE TABLE annotation_runs(id INTEGER PRIMARY KEY,dataset_id INTEGER,filename TEXT,ms2_sha256 TEXT,registered_ms2_sha256 TEXT,library_sha256 TEXT,lotus_csv_sha256 TEXT,config_sha256 TEXT,config_path TEXT,status TEXT,error TEXT,taxon_name TEXT,taxon_qid TEXT,taxonomy_query TEXT,matcher_version TEXT,created_at TEXT);
        CREATE VIEW annotation_current_runs AS SELECT * FROM annotation_runs;
        CREATE TABLE annotation_features(run_id INTEGER,feature_id TEXT,precursor_mz REAL,status TEXT);
        CREATE TABLE annotation_hits(run_id INTEGER,feature_id TEXT,rank INTEGER,library_spectrum_id TEXT,library_inchikey TEXT,spectral_score REAL,taxonomic_score REAL,combined_score REAL,rank_before_taxonomy INTEGER,taxonomic_support TEXT,matches INTEGER,ms1_deviation_ppm REAL,matched_organism_name TEXT,matched_organism_wikidata TEXT,matched_shared_rank TEXT,matched_lotus_inchikey TEXT,matched_lotus_structure_wikidata TEXT,matched_lotus_reference_wikidata TEXT,matched_lotus_reference_doi TEXT);
        CREATE TABLE chemical_lookups(inchikey TEXT,status TEXT);
        CREATE TABLE chemical_entities(inchikey TEXT,wikidata_qid TEXT);
        INSERT INTO taxa VALUES(1,'Salvia officinalis');
        INSERT INTO taxon_resolutions VALUES(1,'Q111');
        INSERT INTO sample_rows VALUES(1,1,'A',1,'{\"sample_id\":\"A\"}'),(1,2,'B',1,'{\"sample_id\":\"B\"}');
        INSERT INTO spectrum_files VALUES(1,'run.mzML',1,'raw/run.mzML'),(1,'missing.mzML',2,'raw/missing.mzML');
        INSERT INTO annotation_features VALUES(1,'1',123.45,'hits');
        INSERT INTO annotation_hits VALUES(1,'1',1,'reference-1','AAAAAAAAAAAAAA-BBBBBBBBBB-C',0.8,9,9.8,1,'supported',8,1.25,'Salvia officinalis','http://www.wikidata.org/entity/Q111','species',NULL,NULL,NULL,NULL);
        INSERT INTO chemical_lookups VALUES('AAAAAAAAAAAAAA-BBBBBBBBBB-C','matched');
        INSERT INTO chemical_entities VALUES('AAAAAAAAAAAAAA-BBBBBBBBBB-C','Q222');").unwrap();
    db.execute("INSERT INTO datasets VALUES (1,'10.1234/graph',?1,?2)", params![dataset.to_str().unwrap(), metadata.to_str().unwrap()]).unwrap();
    db.execute("INSERT INTO processed_spectra VALUES (1,'run.mzML',?1,?2,?3,NULL)", params![mgf.to_str().unwrap(), sha(mgf_bytes), audit.to_str().unwrap()]).unwrap();
    db.execute("INSERT INTO annotation_runs VALUES (1,1,'run.mzML',?1,?1,'library-digest','lotus-digest',?2,?3,'hits',NULL,'Salvia officinalis','Q111','Salvia officinalis','spectral-matcher/0.1.0','2025-01-01T00:00:00Z')", params![sha(mgf_bytes), sha(&fs::read(&matcher).unwrap()), matcher.to_str().unwrap()]).unwrap();
    db.execute("INSERT INTO annotation_runs VALUES (2,1,'missing.mzML',NULL,NULL,'library-digest','lotus-digest','matcher-digest',?1,'error','taxon missing from LOTUS','Salvia officinalis','Q111','Salvia officinalis','spectral-matcher/0.1.0','2025-01-01T00:00:00Z')", [matcher.to_str().unwrap()]).unwrap();
    drop(db);

    let config = root.join("job.toml");
    let flock = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    fs::write(&config, format!("workspace = {:?}\nflock_root = {:?}\n[pipeline]\nstages = [\"kg_export\", \"kg_validate\"]\ntop_n_stages = [\"kg_export\", \"kg_validate\"]\n[dataset]\nroot = \"dataset\"\ndoi = \"10.1234/graph\"\nmetadata = \"metadata.tsv\"\nms_dir = \"raw\"\ntaxon_column = \"source_taxon\"\n[database]\npath = \"production.sqlite\"\ntop_n_root = \"top-n\"\n[picoto]\noutput_dir = \"picoto\"\njobs = 1\n[indexes]\ntaxon_index = \"taxa.sqlite\"\nchemical_index = \"chemicals.sqlite\"\n[annotation]\nlibrary = \"unused.mgf\"\nlotus_csv = \"unused.csv\"\nsettings = \"matcher.toml\"\n[knowledge_graph]\noutput_dir = \"graphs\"\nallow_partial = true\n", root.display().to_string(), flock.display().to_string())).unwrap();
    (temp, config)
}

#[test]
fn partial_top_n_job_exports_a_queryable_graph_without_claiming_complete_annotation() {
    let (temp, config) = partial_fixture();
    let root = temp.path();
    let output = Command::new(env!("CARGO_BIN_EXE_mato"))
        .args(["run", "--config"])
        .arg(&config)
        .args(["--topN", "2"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "failed annotations must remain visible: {stderr}");
    assert!(stderr.contains("annotation files require reprocessing"), "{stderr}");
    let graph_dir = root.join("graphs/2");
    let graphs = fs::read_dir(graph_dir).expect("partial graph was not exported")
        .map(|entry| entry.unwrap().path()).collect::<Vec<_>>();
    assert_eq!(graphs.len(), 1);
    assert!(graphs[0].file_name().unwrap().to_string_lossy().starts_with("graph-partial-"));
    let graph = fs::read_to_string(&graphs[0]).unwrap();
    assert!(graph.contains("http://www.wikidata.org/entity/Q111") && graph.contains("http://www.wikidata.org/entity/Q222"), "KG lost Wikidata-linked evidence");
    assert!(String::from_utf8_lossy(&output.stdout).contains("SHACL conforms"), "partial graph was not validated: {stderr}");
    let report = fs::read_dir(root.join("top-n/2/reports")).unwrap().next().unwrap().unwrap().path();
    let contents = fs::read_to_string(report).unwrap();
    assert!(contents.contains("Status: partial") && contents.contains(graphs[0].to_str().unwrap()), "{contents}");
}

#[test]
fn partial_annotation_without_permission_does_not_publish_a_graph() {
    let (temp, config) = partial_fixture();
    let current = fs::read_to_string(&config).unwrap();
    fs::write(&config, current.replace("allow_partial = true", "allow_partial = false")).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_mato"))
        .args(["run", "--config"]).arg(&config)
        .args(["--topN", "2"]).output().unwrap();
    assert!(!output.status.success(), "failed annotations must not be called complete");
    assert!(!temp.path().join("graphs/2").exists(), "no partial graph may be published by default");
    let report = fs::read_dir(temp.path().join("top-n/2/reports"))
        .unwrap().next().unwrap().unwrap().path();
    let contents = fs::read_to_string(report).unwrap();
    assert!(contents.contains("Status: partial") && contents.contains("Knowledge graph: Not produced."), "{contents}");
}
