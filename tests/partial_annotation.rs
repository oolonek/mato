use std::fs;
use std::path::Path;
use std::process::Command;

use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};

const QUERY: &str =
    "BEGIN IONS\nFEATURE_ID=1\nTITLE=query\nPEPMASS=100\nCHARGE=1+\n10 100\n20 80\nEND IONS\n";
const LIBRARY: &str = "BEGIN IONS\nNAME=Datura\nINCHIKEY=UZLXIIADBIXQHY-UHFFFAOYSA-N\nPEPMASS=100\nCHARGE=1+\n10 100\n20 80\nEND IONS\n";
const LOTUS: &str = "structure_inchikey,organism_wikidata,organism_name,organism_taxonomy_01domain,organism_taxonomy_02kingdom,organism_taxonomy_03phylum,organism_taxonomy_04class,organism_taxonomy_05order,organism_taxonomy_06family,organism_taxonomy_07tribe,organism_taxonomy_08genus,organism_taxonomy_09species,organism_taxonomy_10varietas\nUZLXIIADBIXQHY-JNUSPZCGSA-N,http://www.wikidata.org/entity/Q1,Datura stramonium,Eukaryota,Archaeplastida,Streptophyta,Magnoliopsida,Solanales,Solanaceae,NA,Datura,Datura stramonium,NA\n";
const SETTINGS: &str = "library_polarity = 'positive'\n[parse]\nquery_identifier = 'FEATURE_ID'\nlibrary_identifier = 'INCHIKEY'\nmin_peaks = 2\nmax_peaks = 100\nmissing_precursor_mz = 'reject'\n[search]\nmetric = 'HungarianCosine'\nprecursor_mz_tolerance = 0.02\nfragment_mz_tolerance = 0.02\nmz_power = 0.0\nintensity_power = 1.0\ntop_n_peaks = 10\nmin_matched_peaks = 2\nmin_similarity_threshold = 0.1\ntop_n = 3\n";

#[test]
fn recorded_annotation_errors_allow_enrichment_and_retry_after_taxon_resolution() {
    let temporary = tempfile::tempdir().unwrap();
    let dir = temporary.path();
    let dataset = dir.join("dataset");
    fs::create_dir_all(dataset.join("raw")).unwrap();
    fs::write(
        dataset.join("metadata.tsv"),
        "taxon\nDatura stramonium\nMissing species\n",
    )
    .unwrap();
    fs::write(dir.join("library.mgf"), LIBRARY).unwrap();
    fs::write(dir.join("lotus.csv"), LOTUS).unwrap();
    fs::write(dir.join("settings.toml"), SETTINGS).unwrap();
    let ms2 = dir.join("query.mgf");
    fs::write(&ms2, QUERY).unwrap();
    let digest = format!("{:x}", Sha256::digest(QUERY.as_bytes()));
    let db_path = dir.join("flock.sqlite");
    let db = Connection::open(&db_path).unwrap();
    db.execute_batch(
        "CREATE TABLE datasets(id INTEGER PRIMARY KEY,doi TEXT,root TEXT,metadata_path TEXT,ms_dir TEXT);
         CREATE TABLE taxa(id INTEGER PRIMARY KEY,name TEXT);
         CREATE TABLE taxon_resolutions(taxon_id INTEGER PRIMARY KEY,qid TEXT,catalogue_of_life_id TEXT,error TEXT);
         CREATE TABLE sample_rows(dataset_id INTEGER,row_number INTEGER,sample_id TEXT,taxon_id INTEGER,attributes_json TEXT,PRIMARY KEY(dataset_id,row_number));
         CREATE TABLE spectrum_files(dataset_id INTEGER,filename TEXT,row_number INTEGER,column_name TEXT,relative_path TEXT,PRIMARY KEY(dataset_id,filename));
         CREATE TABLE processed_spectra(dataset_id INTEGER,filename TEXT,ms2_path TEXT NOT NULL,ms2_sha256 TEXT NOT NULL,features_path TEXT NOT NULL,paired_path TEXT,registered_at TEXT,PRIMARY KEY(dataset_id,filename));
         CREATE TABLE chemical_lookups(inchikey TEXT PRIMARY KEY,status TEXT NOT NULL,checked_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')));
         INSERT INTO chemical_lookups(inchikey,status) VALUES('UZLXIIADBIXQHY-UHFFFAOYSA-N','matched');
         INSERT INTO taxa VALUES(1,'Datura stramonium'),(2,'Missing species');
         INSERT INTO taxon_resolutions VALUES(1,'Q1',NULL,NULL),(2,'Q999',NULL,NULL);
         INSERT INTO sample_rows VALUES(1,1,'good',1,'{\"sample_type\":\"sample\",\"ionization_mode\":\"positive\"}'),
                                       (1,2,'missing',2,'{\"sample_type\":\"sample\",\"ionization_mode\":\"positive\"}');
         INSERT INTO spectrum_files VALUES(1,'good.mzML',1,'positive_filename','good.mzML'),
                                          (1,'missing.mzML',2,'positive_filename','missing.mzML');",
    )
    .unwrap();
    db.execute(
        "INSERT INTO datasets VALUES(1,'doi',?1,?2,?3)",
        params![
            dataset.to_str().unwrap(),
            dataset.join("metadata.tsv").to_str().unwrap(),
            dataset.join("raw").to_str().unwrap()
        ],
    )
    .unwrap();
    for name in ["good.mzML", "missing.mzML"] {
        db.execute("INSERT INTO processed_spectra(dataset_id,filename,ms2_path,ms2_sha256,features_path) VALUES(1,?1,?2,?3,?2)", params![name, ms2.to_str().unwrap(), digest]).unwrap();
    }
    let config = dir.join("job.toml");
    let indexes_config = dir.join("wikidata.toml");
    fs::write(&indexes_config,
        "[indexes]\ntaxon_index = \"taxa.sqlite\"\nchemical_index = \"chemicals.sqlite\"\n"
    ).unwrap();
    let chemicals = Connection::open(dir.join("chemicals.sqlite")).unwrap();
    chemicals.execute_batch(
        "CREATE TABLE wikidata_chemical_snapshot(
            id INTEGER PRIMARY KEY,generation INTEGER NOT NULL,fetched_at TEXT NOT NULL,
            endpoint TEXT NOT NULL,pairs INTEGER NOT NULL);
         INSERT INTO wikidata_chemical_snapshot VALUES(1,1,'2026-01-01','fixture',1);
         CREATE TABLE wikidata_chemical_index(inchikey TEXT NOT NULL,wikidata_qid TEXT NOT NULL,
            name TEXT,PRIMARY KEY(inchikey,wikidata_qid)) WITHOUT ROWID;
         CREATE TABLE wikidata_chemical_cids(inchikey TEXT NOT NULL,wikidata_qid TEXT NOT NULL,
            pubchem_cid TEXT NOT NULL,PRIMARY KEY(inchikey,wikidata_qid,pubchem_cid)) WITHOUT ROWID;
         INSERT INTO wikidata_chemical_index
            VALUES('UZLXIIADBIXQHY-UHFFFAOYSA-N','Q888','reference candidate');"
    ).unwrap();
    let flock_root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    fs::write(&config, format!(
        "flock_root = {:?}\nwikidata_config = \"wikidata.toml\"\n[pipeline]\nstages = [\"annotate\", \"chemical_enrich\"]\n[dataset]\nroot = {:?}\ndoi = \"doi\"\nmetadata = \"metadata.tsv\"\nms_dir = \"raw\"\ntaxon_column = \"taxon\"\n[database]\npath = {:?}\n[picoto]\noutput_dir = {:?}\njobs = 1\n[annotation]\nlibrary = {:?}\nlotus_csv = {:?}\nsettings = {:?}\n",
        flock_root.display().to_string(), dataset.display().to_string(), db_path.display().to_string(), dir.join("picoto").display().to_string(), dir.join("library.mgf").display().to_string(), dir.join("lotus.csv").display().to_string(), dir.join("settings.toml").display().to_string()
    )).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_mato"))
        .args(["run", "--config"])
        .arg(&config)
        .output()
        .unwrap();
    let log = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "partial annotations must remain visible: {log}"
    );
    assert!(
        log.contains("annotation files require reprocessing"),
        "partial result lost: {log}"
    );
    let statuses: Vec<String> = db
        .prepare("SELECT status FROM annotation_runs ORDER BY filename")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(statuses, ["hits", "error"]);
    let candidate_qid: String = db.query_row(
        "SELECT wikidata_qid FROM chemical_entities WHERE inchikey='UZLXIIADBIXQHY-UHFFFAOYSA-N'",
        [], |row| row.get(0)
    ).unwrap();
    assert_eq!(candidate_qid, "Q888", "chemical enrichment must resolve the saved current hit");
    let report_dir = dir.join("reports");
    let first_report = fs::read_dir(&report_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .next()
        .unwrap();
    let partial_report = fs::read_to_string(&first_report).unwrap();
    assert!(
        partial_report.contains("Status: partial"),
        "{partial_report}"
    );
    assert!(
        partial_report.contains("| Ingested sample records | 2 |"),
        "{partial_report}"
    );
    assert!(
        partial_report.contains("| Outstanding failed files | 1 |"),
        "{partial_report}"
    );
    assert!(
        partial_report.contains("missing.mzML") && partial_report.contains("LOTUS"),
        "{partial_report}"
    );
    assert!(
        partial_report.contains("show annotation_runs")
            && partial_report.contains(config.to_str().unwrap()),
        "{partial_report}"
    );

    db.execute("UPDATE taxa SET name='Datura stramonium' WHERE id=2", [])
        .unwrap();
    db.execute("UPDATE taxon_resolutions SET qid='Q1' WHERE taxon_id=2", [])
        .unwrap();
    let resumed = Command::new(env!("CARGO_BIN_EXE_mato"))
        .args(["run", "--config"])
        .arg(&config)
        .output()
        .unwrap();
    assert!(
        resumed.status.success(),
        "{}",
        String::from_utf8_lossy(&resumed.stderr)
    );
    let current: Vec<(String, String)> = db
        .prepare("SELECT filename,status FROM annotation_current_runs ORDER BY filename")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        current,
        [
            ("good.mzML".into(), "hits".into()),
            ("missing.mzML".into(), "hits".into())
        ]
    );
    let after = fs::read_dir(&report_dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    assert_eq!(after.len(), 2);
    let latest = after.iter().find(|path| **path != first_report).unwrap();
    let complete_report = fs::read_to_string(latest).unwrap();
    assert!(
        complete_report.contains("Status: complete"),
        "{complete_report}"
    );
    assert!(
        complete_report.contains("No outstanding failed files"),
        "{complete_report}"
    );
    assert!(
        complete_report.contains("| Outstanding failed files | 0 |"),
        "{complete_report}"
    );
    assert_eq!(fs::read_to_string(first_report).unwrap(), partial_report);
}
