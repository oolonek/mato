#!/usr/bin/env python3
"""Run the first 100 MassIVE metadata rows in place, then enrich candidate structures."""

import argparse
from pathlib import Path
import sqlite3
import subprocess
import sys
import tomllib

MATO = Path(__file__).resolve().parents[1]
SUBSET_CONFIG = MATO / "config/massive-100.toml"
MAX_ROWS = 100


def has_current_hits(database: Path, dataset_root: Path) -> bool:
    if not database.is_file():
        return False
    with sqlite3.connect(database.as_uri() + "?mode=ro", uri=True) as connection:
        if not connection.execute(
            "SELECT 1 FROM sqlite_master WHERE type='view' AND name='annotation_current_runs'"
        ).fetchone():
            return False
        return connection.execute(
            "SELECT 1 FROM annotation_current_runs r JOIN datasets d ON d.id=r.dataset_id "
            "JOIN annotation_hits h ON h.run_id=r.id "
            "WHERE d.root=? AND r.status='hits' AND h.library_inchikey IS NOT NULL "
            "AND trim(h.library_inchikey)<>'' LIMIT 1",
            (str(dataset_root.resolve()),),
        ).fetchone() is not None


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.parse_args()

    with SUBSET_CONFIG.open("rb") as file:
        subset = tomllib.load(file)
    database = Path(subset["database"]["path"])
    dataset_root = Path(subset["dataset"]["root"])

    try:
        result = subprocess.run([
            "cargo", "run", "--release", "--manifest-path", str(MATO / "Cargo.toml"),
            "--", "run", "--config", str(SUBSET_CONFIG), "--max-rows", str(MAX_ROWS),
        ], check=False)
        exit_code = result.returncode
    except OSError as error:
        print(f"Mato could not launch: {error}", file=sys.stderr)
        return 1

    try:
        enrich = exit_code == 0 or has_current_hits(database, dataset_root)
    except sqlite3.Error as error:
        print(f"Cannot inspect saved annotations for Jaiba: {error}", file=sys.stderr)
        enrich = False
    if enrich:
        try:
            jaiba = subprocess.run([
                "cargo", "run", "--release", "--manifest-path", str(MATO.parent / "jaiba/Cargo.toml"),
                "--", "enrich", "--db", str(database),
            ], check=False)
            if jaiba.returncode:
                print(f"Jaiba stopped with incomplete chemical lookups (exit {jaiba.returncode}).", file=sys.stderr)
                if not exit_code:
                    exit_code = jaiba.returncode
        except OSError as error:
            print(f"Jaiba could not launch: {error}", file=sys.stderr)
            if not exit_code:
                exit_code = 1
    elif exit_code:
        print("Jaiba skipped: no current annotation candidates for this dataset.", file=sys.stderr)

    if exit_code:
        print(f"🦎 Rehearsal stopped with incomplete results (exit {exit_code}). Inspect saved results with: python3 {MATO / 'scripts/inspect_massive_20.py'} --config {SUBSET_CONFIG}", file=sys.stderr)
    return exit_code


if __name__ == "__main__":
    raise SystemExit(main())
