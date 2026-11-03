#!/usr/bin/env python3
"""Show a truthful, read-only Mato demo report with SQL/xan and Picoto evidence."""

import argparse
import csv
from contextlib import closing
import io
import json
from pathlib import Path
import re
import shlex
import shutil
import sqlite3
import subprocess
import sys
import tomllib
from urllib.parse import quote

CONFIG = Path(__file__).resolve().parents[1] / "config/massive-20.toml"
VIEW = ["xan", "view", "--hide-index", "--hide-info", "--repeat-headers", "never"]


def require_tools() -> None:
    for tool in ("sqlite3", "xan"):
        if shutil.which(tool) is None:
            raise RuntimeError(f"demo report needs {tool} on PATH")


def table(db: Path, sql: str, title: str, full_sql: str | None = None) -> None:
    print(f"\n{title}", flush=True)
    command = shlex.join(["sqlite3", "-readonly", "-header", "-csv", str(db),
                          " ".join((full_sql or sql).split())])
    print(f"Full table: {command} | xan view -A -e -p --hide-index", flush=True)
    query = subprocess.Popen(
        ["sqlite3", "-readonly", "-header", "-csv", str(db), sql],
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    )
    assert query.stdout is not None
    shown = subprocess.run(VIEW, stdin=query.stdout, check=False)
    query.stdout.close()
    error = query.stderr.read().decode() if query.stderr else ""
    if query.wait() or shown.returncode:
        raise RuntimeError(f"SQL/xan report failed: {error.strip() or shown.returncode}")


def wikidata_qid(value: str | None) -> str | None:
    match = re.fullmatch(r"(?:https?://(?:www\.)?wikidata\.org/(?:entity|wiki)/)?(Q[1-9]\d*)",
                         value or "")
    return match[1] if match else None


def lotus_rows_for_short_keys(lotus_csv: Path, short_keys: set[str]) -> dict[str, list[dict[str, str]]]:
    """Fetch both candidate and biosource structures in one local CSV scan."""
    if not short_keys or not lotus_csv.is_file():
        return {}
    keys = sorted(short_keys)
    args = ["xan", "search", "-s", "structure_inchikey", keys[0]]
    for key in keys[1:]:
        args.extend(("-P", key))
    found = subprocess.Popen(args + [str(lotus_csv)], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    assert found.stdout is not None
    selected = subprocess.run(
        ["xan", "select", "structure_inchikey,structure_wikidata,organism_wikidata,organism_name"],
        stdin=found.stdout, capture_output=True, text=True, check=False,
    )
    found.stdout.close()
    error = found.stderr.read().decode() if found.stderr else ""
    if found.wait() or selected.returncode:
        raise RuntimeError(f"LOTUS structure lookup failed: {error.strip() or selected.stderr.strip()}")
    by_short: dict[str, list[dict[str, str]]] = {}
    for row in csv.DictReader(io.StringIO(selected.stdout)):
        full_key = row["structure_inchikey"]
        short_key = full_key[:14]
        if short_key in short_keys and full_key.startswith(short_key + "-"):
            by_short.setdefault(short_key, []).append(row)
    return by_short


def linked_pair_table(rows: list[sqlite3.Row], lotus_csv: Path) -> None:
    short_keys = {short for row in rows
                  for short in (row["matched_short_inchikey"],
                                row["library_inchikey"][:14] if row["library_inchikey"] else None)
                  if short}
    sources = lotus_rows_for_short_keys(lotus_csv, short_keys)
    tty = sys.stdout.isatty()
    headers = ("Species", "Candidate", "Tax", "Species WD", "Candidate WD", "Source WD", "Basis")
    values = []
    for row in rows:
        key = row["library_inchikey"]
        short = row["matched_short_inchikey"] or (key[:14] if key else None)
        records = sources.get(short, [])
        candidate_qids = sorted({qid for record in sources.get(key[:14] if key else None, [])
                                 if record["structure_inchikey"] == key
                                 if (qid := wikidata_qid(record["structure_wikidata"]))})
        organism_qid = wikidata_qid(row["matched_organism_wikidata"])
        if organism_qid:
            matched = [record for record in records
                       if wikidata_qid(record["organism_wikidata"]) == organism_qid]
        else:
            organism_name = row["matched_organism_name"]
            matched = [record for record in records
                       if organism_name and record["organism_name"] == organism_name]
        source_keys = sorted({record["structure_inchikey"] for record in matched})
        source_qids = sorted({qid for record in matched
                              if (qid := wikidata_qid(record["structure_wikidata"]))})
        basis = "exact-key" if key in source_keys else "2D-only" if source_keys else "no source"
        species_qid = wikidata_qid(row["taxon_qid"])
        species_url = f"https://www.wikidata.org/wiki/{species_qid}" if species_qid else None
        if len(candidate_qids) == 1:
            candidate_label = candidate_qids[0]
            candidate_url = f"https://www.wikidata.org/wiki/{candidate_label}"
        elif key:
            candidate_label = "SEARCH" if not candidate_qids else "AMBIGUOUS"
            candidate_url = f"https://www.wikidata.org/w/index.php?search={quote(key)}&title=Special:Search"
        else:
            candidate_label, candidate_url = "-", None
        source_urls = [(qid, f"https://www.wikidata.org/wiki/{qid}") for qid in source_qids]
        labels = (
            row["taxon_name"], row["library_name"], f"{row['taxonomic_score']:g}",
            species_qid if tty and species_url else species_url or "-",
            candidate_label if tty else candidate_url or "-",
            ",".join(source_qids if tty else (url for _, url in source_urls)) or "-",
            basis,
        )
        values.append({"labels": labels, "species_url": species_url, "candidate_url": candidate_url,
                       "source_urls": source_urls, "candidate_key": key, "source_keys": source_keys})

    print(f"\n🔗 Wikidata links for {len(values)} best species–chemical pairs "
          "(open or Ctrl-click linked QIDs in your terminal)")
    widths = [max(len(header), *(len(item["labels"][index]) for item in values))
              for index, header in enumerate(headers)]

    def cell(label: str, url: str | None, width: int) -> str:
        padded = label.ljust(width)
        return f"\x1b]8;;{url}\x1b\\{padded}\x1b]8;;\x1b\\" if tty and url else padded

    print(" | ".join(header.ljust(width) for header, width in zip(headers, widths)))
    print("-+-".join("-" * width for width in widths))
    for item in values:
        labels = item["labels"]
        shown = [cell(labels[index], None, widths[index]) for index in range(len(headers))]
        shown[3] = cell(labels[3], item["species_url"], widths[3])
        shown[4] = cell(labels[4], item["candidate_url"], widths[4])
        if tty and item["source_urls"]:
            shown[5] = ",".join(cell(qid, url, len(qid)) for qid, url in item["source_urls"])
            shown[5] += " " * (widths[5] - len(labels[5]))
        print(" | ".join(shown))
    print("Candidate WD uses the library's exact full InChIKey; Source WD is a LOTUS structure "
          "linked to the matched biosource via the first 14 characters.")
    print("2D-only: that biosource has no record for the candidate's exact full key. "
          "The taxonomic bonus does not establish occurrence of the linked candidate.")
    mismatch = next((item for item in values if item["labels"][6] == "2D-only"), None)
    if mismatch:
        print(f"Example {mismatch['labels'][0]}: library {mismatch['candidate_key']}; "
              f"matching LOTUS biosource key(s) {', '.join(mismatch['source_keys'])}.")


def feature_audit(path: Path, feature_id: str) -> None:
    print(f"Full table: xan view -d $'\\t' -A -e -p --hide-index {shlex.quote(str(path))}", flush=True)
    expression = "feature_id eq " + json.dumps(feature_id)
    filtered = subprocess.Popen(
        ["xan", "filter", "-d", "\t", "-l", "1", expression, str(path)],
        stdout=subprocess.PIPE,
    )
    assert filtered.stdout is not None
    selected = subprocess.Popen(
        ["xan", "select", "feature_id,mz,rt_seconds,quality_score,ms2_scan_ids"],
        stdin=filtered.stdout, stdout=subprocess.PIPE,
    )
    filtered.stdout.close()
    assert selected.stdout is not None
    shown = subprocess.run(VIEW, stdin=selected.stdout, check=False)
    selected.stdout.close()
    codes = (filtered.wait(), selected.wait(), shown.returncode)
    if any(codes):
        raise RuntimeError(f"could not inspect feature {feature_id} in {path}")


def mgf_preview(path: Path, feature_id: str) -> None:
    """Show only the selected MS2 headers and four actual fragment peaks."""
    with path.open(encoding="utf-8") as file:
        block = False
        found = False
        peaks = 0
        details = []
        for line in file:
            value = line.strip()
            if value == "BEGIN IONS":
                block, found, peaks, details = True, False, 0, [value]
            elif block and value == f"FEATURE_ID={feature_id}":
                found = True
                details.append(value)
            elif block and found and value == "END IONS":
                if peaks > 4:
                    details.append(f"... {peaks - 4} further peaks")
                print("\n".join(details + [value]))
                return
            elif block and found:
                if value.startswith(("PEPMASS=", "CHARGE=", "RTINSECONDS=", "SCANS=")):
                    details.append(value)
                elif value and "=" not in value:
                    peaks += 1
                    if peaks <= 4:
                        details.append(value)
    raise RuntimeError(f"MS2 spectrum for feature {feature_id} absent from {path}")


def inspect(config_path: Path = CONFIG) -> None:
    require_tools()
    with config_path.open("rb") as file:
        config = tomllib.load(file)
    db = Path(config["database"]["path"])
    root = str(Path(config["dataset"]["root"]).resolve())
    if not db.is_file():
        raise FileNotFoundError(f"demo database not yet created: {db}")

    with closing(sqlite3.connect(db.as_uri() + "?mode=ro", uri=True)) as connection:
        connection.row_factory = sqlite3.Row
        dataset = connection.execute("SELECT id FROM datasets WHERE root=?", (root,)).fetchone()
        if dataset is None:
            raise RuntimeError(f"no ingested dataset for {root} in {db}")
        dataset_id = int(dataset["id"])
        table(db, f"""SELECT
            (SELECT COUNT(*) FROM sample_rows WHERE dataset_id={dataset_id}) AS samples,
            (SELECT COUNT(*) FROM spectrum_files WHERE dataset_id={dataset_id}) AS spectra,
            (SELECT COUNT(*) FROM processed_spectra WHERE dataset_id={dataset_id}) AS registered,
            (SELECT COUNT(DISTINCT t.id) FROM taxa t JOIN sample_rows s ON s.taxon_id=t.id
               WHERE s.dataset_id={dataset_id}) AS taxa""", "🐦 Flock: persisted dataset",
              full_sql=f"SELECT * FROM spectrum_files WHERE dataset_id={dataset_id} ORDER BY filename")

        available = connection.execute("SELECT 1 FROM sqlite_master WHERE type='view' AND name='annotation_current_runs'").fetchone()
        if not available:
            print("Annotation has not started; registration and metadata remain inspectable.")
            return

        table(db, f"""SELECT CASE
            WHEN lower(COALESCE(json_extract(s.attributes_json,'$.sample_type'),'')) LIKE '%qc%'
              OR lower(COALESCE(json_extract(s.attributes_json,'$.sample_type'),'')) LIKE '%blank%'
              OR lower(COALESCE(json_extract(s.attributes_json,'$.sample_type'),'')) LIKE '%quality control%'
                THEN 'QC/blank skipped'
            WHEN lower(json_extract(s.attributes_json,'$.ionization_mode'))='negative'
              OR (json_type(s.attributes_json,'$.ionization_mode') IS NULL
                  AND (instr(lower(f.column_name),'negative')>0
                       OR instr(lower(f.column_name),'_neg')>0))
                THEN 'negative skipped'
            WHEN a.status='hits' THEN 'annotated'
            ELSE COALESCE(a.status,'pending') END AS outcome,
            COUNT(*) AS files, COALESCE(SUM(a.query_count),0) AS queries,
            COALESCE(SUM(a.hit_count),0) AS candidate_hits
            FROM spectrum_files f
            JOIN sample_rows s ON s.dataset_id=f.dataset_id AND s.row_number=f.row_number
            LEFT JOIN annotation_current_runs a ON a.dataset_id=f.dataset_id AND a.filename=f.filename
            WHERE f.dataset_id={dataset_id}
            GROUP BY outcome ORDER BY outcome""", "🦃 Hocofaisan: current per-file outcomes",
              full_sql=f"""SELECT f.filename, f.column_name, a.status, a.error, a.query_count, a.hit_count
                  FROM spectrum_files f LEFT JOIN annotation_current_runs a
                  ON a.dataset_id=f.dataset_id AND a.filename=f.filename
                  WHERE f.dataset_id={dataset_id} ORDER BY f.filename""")

        chemical_lookups = connection.execute(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name='chemical_lookups'"
        ).fetchone()
        if chemical_lookups:
            table(db, f"""WITH keys AS (
                SELECT DISTINCT h.library_inchikey AS inchikey
                FROM annotation_current_runs r
                JOIN annotation_hits h ON h.run_id=r.id
                WHERE r.dataset_id={dataset_id} AND r.status='hits'
                  AND h.library_inchikey IS NOT NULL AND trim(h.library_inchikey)<>''
            )
            SELECT COALESCE(l.status,'pending') AS lookup_status, COUNT(*) AS full_inchikeys
            FROM keys k LEFT JOIN chemical_lookups l ON l.inchikey=k.inchikey
            GROUP BY lookup_status ORDER BY lookup_status""",
                  "🦀 Jaiba: current candidate structures by exact full-key lookup status",
                  full_sql=f"""SELECT DISTINCT h.library_inchikey, l.status,
                      e.wikidata_qid, e.name
                      FROM annotation_current_runs r
                      JOIN annotation_hits h ON h.run_id=r.id
                      LEFT JOIN chemical_lookups l ON l.inchikey=h.library_inchikey
                      LEFT JOIN chemical_entities e ON e.inchikey=h.library_inchikey
                      WHERE r.dataset_id={dataset_id} AND r.status='hits'
                        AND h.library_inchikey IS NOT NULL
                      ORDER BY h.library_inchikey, e.wikidata_qid""")
        else:
            print("Jaiba has not started; no chemical lookup status is stored yet.")

        failures = connection.execute(
            "SELECT taxon_name,error FROM annotation_current_runs WHERE dataset_id=? AND status='error'",
            (dataset_id,),
        ).fetchall()
        if failures:
            table(db, f"""SELECT taxon_name AS taxon, taxon_qid AS qid,
                CASE WHEN error LIKE '%Biosource not found in LOTUS%'
                    THEN 'absent from frozen LOTUS CSV' ELSE error END AS issue
                FROM annotation_current_runs WHERE dataset_id={dataset_id} AND status='error'
                ORDER BY taxon_name""", "⚠ Taxa without completed annotation",
                  full_sql=f"SELECT * FROM annotation_current_runs WHERE dataset_id={dataset_id} AND status='error' ORDER BY taxon_name")
            if all("not found in LOTUS CSV" in row["error"] for row in failures):
                print("These QIDs resolved in Wikidata; their biosources are missing from this LOTUS snapshot. No annotation was fabricated for them.")

        exemplar = connection.execute(f"""SELECT r.taxon_name, r.filename, h.feature_id,
            json_extract(h.library_headers_json,'$.CONFIDENCE') AS library_provenance,
            p.features_path, p.ms2_path, p.paired_path
            FROM annotation_current_runs r
            JOIN annotation_hits h ON h.run_id=r.id AND h.rank=1
            JOIN processed_spectra p ON p.dataset_id=r.dataset_id AND p.filename=r.filename
            WHERE r.dataset_id={dataset_id} AND r.status='hits'
            ORDER BY h.combined_score DESC LIMIT 1""").fetchone()
        if exemplar is None:
            print("No candidate hits yet; feature and MGF previews require a successful annotation.")
            return

        pair_sql = f"""WITH best AS (
            SELECT r.taxon_name AS species, h.library_name AS chemical_candidate,
                COALESCE(h.matched_organism_name,'-') AS lotus_biosource,
                COALESCE(h.matched_shared_rank,'-') AS shared_rank,
                ROUND(h.spectral_score,3) AS spectral,
                h.taxonomic_score AS taxonomic_score,
                ROUND(h.combined_score,3) AS combined,
                h.combined_score AS sort_score,
                ROW_NUMBER() OVER (PARTITION BY r.filename ORDER BY h.combined_score DESC) AS n
            FROM annotation_current_runs r
            JOIN annotation_hits h ON h.run_id=r.id AND h.rank=1
            WHERE r.dataset_id={dataset_id} AND r.status='hits')
            SELECT species,chemical_candidate,lotus_biosource,shared_rank,spectral,taxonomic_score,combined
            FROM best WHERE n=1 ORDER BY sort_score DESC"""
        table(db, pair_sql + " LIMIT 20", "🔎 Species–candidate pairs (best per file; combined = spectral + taxonomic)",
              full_sql=f"""SELECT r.taxon_name AS species, h.library_name AS chemical_candidate,
                  h.matched_organism_name AS lotus_biosource, h.matched_shared_rank AS shared_rank,
                  h.spectral_score, h.taxonomic_score, h.combined_score
                  FROM annotation_current_runs r JOIN annotation_hits h ON h.run_id=r.id
                  WHERE r.dataset_id={dataset_id} AND r.status='hits'
                  ORDER BY r.taxon_name, h.feature_id, h.rank""")
        print("Shared rank 'species' gives a 9-point LOTUS lineage bonus (genus gives 8) via the 14-character InChIKey connectivity block; it does not verify the exact library structure.")

        linked_rows = connection.execute(f"""WITH best AS (
            SELECT r.taxon_name, r.taxon_qid, h.library_name, h.library_inchikey,
                h.matched_short_inchikey, h.matched_organism_name, h.matched_organism_wikidata,
                h.taxonomic_score, h.combined_score,
                ROW_NUMBER() OVER (PARTITION BY r.filename ORDER BY h.combined_score DESC) AS n
            FROM annotation_current_runs r
            JOIN annotation_hits h ON h.run_id=r.id AND h.rank=1
            WHERE r.dataset_id={dataset_id} AND r.status='hits')
            SELECT taxon_name,taxon_qid,library_name,library_inchikey,matched_short_inchikey,
                matched_organism_name,matched_organism_wikidata,taxonomic_score
            FROM best WHERE n=1 ORDER BY combined_score DESC LIMIT 20""").fetchall()
        linked_pair_table(linked_rows, Path(config["annotation"]["lotus_csv"]))

        feature_id = str(exemplar["feature_id"])
        audit = Path(exemplar["features_path"])
        ms2 = Path(exemplar["ms2_path"])
        print(f"\n🦈 Picoto evidence for {exemplar['taxon_name']} / feature {feature_id}")
        print(f"Audit: {audit}\nMS2-only: {ms2}\nPaired MS1/MS2: {exemplar['paired_path']}", flush=True)
        feature_audit(audit, feature_id)
        mgf_preview(ms2, feature_id)
        print(f"Library provenance: {exemplar['library_provenance'] or 'not recorded'}. A high score is a candidate match, not a confirmed structure.")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--config", type=Path, default=CONFIG, help="Mato TOML with an ingested dataset")
    args = parser.parse_args()
    try:
        inspect(args.config)
    except (OSError, sqlite3.Error, RuntimeError) as error:
        parser.exit(1, f"Demo report unavailable: {error}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
