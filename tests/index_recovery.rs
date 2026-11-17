use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use rusqlite::{Connection, OpenFlags};

#[test]
fn hot_journal_writer() {
    let Ok(path) = env::var("MATO_INDEX_WRITER") else {
        return;
    };
    let db = Connection::open(path).unwrap();
    db.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA cache_size=1; BEGIN IMMEDIATE; UPDATE wikidata_taxon_names SET name='uncommitted';").unwrap();
    // Spill a dirty page so SQLite writes a *hot* rollback journal rather than
    // merely leaving a transaction with an unflushed, cold journal header.
    db.execute(
        "INSERT INTO wikidata_taxon_names(name,qid) VALUES(?1,'Q9')",
        [&"x".repeat(128 * 1024)],
    ).unwrap();
    println!("INDEX_WRITER_READY");
    std::io::stdout().flush().unwrap();
    std::thread::park();
}

#[test]
fn interrupted_refresh_requires_a_writable_recovery_before_read_only_enrichment() {
    let temporary = tempfile::tempdir().unwrap();
    let workspace = temporary.path().join("workspace");
    let indexes = workspace.join("mato/wikidata");
    fs::create_dir_all(&indexes).unwrap();
    let file = indexes.join("taxa.sqlite");
    let db = Connection::open(&file).unwrap();
    db.execute_batch("CREATE TABLE wikidata_taxon_names(name TEXT NOT NULL, qid TEXT NOT NULL);\
        CREATE TABLE wikidata_taxon_snapshot(id INTEGER PRIMARY KEY, generation INTEGER NOT NULL, names INTEGER NOT NULL, endpoint TEXT NOT NULL);\
        INSERT INTO wikidata_taxon_names VALUES('Datura stramonium','Q30959');\
        INSERT INTO wikidata_taxon_snapshot VALUES(1,1,1,'https://qlever.dev/api/wikidata');")
        .unwrap();
    drop(db);

    let mut child = Command::new(env::current_exe().unwrap())
        .args(["--exact", "hot_journal_writer", "--nocapture"])
        .env("MATO_INDEX_WRITER", &file)
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut line = String::new();
    loop {
        line.clear();
        assert!(output.read_line(&mut line).unwrap() > 0, "writer did not open transaction");
        if line.contains("INDEX_WRITER_READY") { break; }
    }
    child.kill().unwrap();
    child.wait().unwrap();
    assert!(file.with_extension("sqlite-journal").exists());

    let ro = Connection::open_with_flags(&file, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let error = ro.query_row("SELECT name FROM wikidata_taxon_names", [], |row| row.get::<_, String>(0)).unwrap_err();
    assert!(error.to_string().contains("readonly database"), "{error:#}");
    drop(ro);

    let config = temporary.path().join("toy.toml");
    let flock = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let template = include_str!("../config/toy.toml");
    fs::write(&config, template
        .replace("/path/to/workspace", workspace.to_str().unwrap())
        .replace("flock_root = \"../..\"", &format!("flock_root = \"{}\"", flock.display())))
        .unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_mato"))
        .args(["indexes", "recover", "--config"])
        .arg(&config)
        .args(["--only", "taxon"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let ro = Connection::open_with_flags(&file, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let name: String = ro.query_row("SELECT name FROM wikidata_taxon_names", [], |row| row.get(0)).unwrap();
    assert_eq!(name, "Datura stramonium");
    let generation: i64 = ro.query_row("SELECT generation FROM wikidata_taxon_snapshot WHERE id=1", [], |row| row.get(0)).unwrap();
    assert_eq!(generation, 1);
    assert!(!file.with_extension("sqlite-journal").exists());
}
