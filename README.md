# mato

Mato runs an ordered flock pipeline from one TOML file. It builds the selected sibling Rust species once per invocation, then calls their existing CLIs without a shell. Child progress and logs stay visible; mato stops on the first failed stage or raw-file registration. The species retain responsibility for validation, storage, taxonomy and annotation.

## From A to Z on the Zenodo toy dataset

The extracted ENPKG toy dataset already exists under `/media/nvme8tb/isla-gabriela/sapayoa/zenodo-10018590`. Its three metadata rows reference six mzXML files. Run all six selected stages—machaca metadata/raw validation, peregrino metadata/spectra/taxa ingestion, cunaguaro Wikidata name resolution with stored IDs/errors, Picoto feature/MGF extraction, peregrino per-file registration and hocofaisan positive-mode annotation with LOTUS taxonomy—with one command from any directory:

```sh
cargo run --release --manifest-path /home/allardpm/git_repos/oolonek/mato/Cargo.toml -- run --config /home/allardpm/git_repos/oolonek/mato/config/toy.toml
```

`config/toy.toml` writes a **separate test** database at `/media/nvme8tb/isla-gabriela/mato/toy.sqlite` and Picoto output beneath `/media/nvme8tb/isla-gabriela/mato/toy-picoto`; it does not change the downloaded toy data or `/media/nvme8tb/isla-gabriela/peregrino/flock.sqlite`. Mato creates the needed output directories through the existing species. Cunaguaro needs live Wikidata access; the test dataset's biological sample has the taxon *Datura stramonium*. QC/blank rows and negative-mode spectra are skipped by hocofaisan, not removed from the ingested database. The library and LOTUS references in the TOML are already stored under `/media/nvme8tb/isla-gabriela/hocofaisan/references/`. To inspect the result:

```sh
cargo run --release --manifest-path /home/allardpm/git_repos/oolonek/peregrino/Cargo.toml -- show annotation_current_runs --db /media/nvme8tb/isla-gabriela/mato/toy.sqlite --limit 20
```

## Hundred-file MassIVE rehearsal

The original 20-file rehearsal under `massive-20/` remains untouched. This command still uses the legacy `run_massive_20.py` filename, but now prepares a **separate 100-file** subset under `massive-100/`. Stop any process using the new subset database before resetting it; remove **only** its database and SQLite sidecars, never the raw copies:

```sh
rm -f -- /media/nvme8tb/isla-gabriela/mato/massive-100/flock.sqlite \
  /media/nvme8tb/isla-gabriela/mato/massive-100/flock.sqlite-wal \
  /media/nvme8tb/isla-gabriela/mato/massive-100/flock.sqlite-shm
```

Run the preparation and pipeline (the inspection report remains a separate command):

```sh
python3 /home/allardpm/git_repos/oolonek/mato/scripts/run_massive_20.py
```

The script selects the first 80 positive-mode and 20 negative-mode `plant specimen` rows with 100 distinct `accepted_exact` taxon names, retaining all original metadata columns. The selected 100 source files total 4,557,167,130 bytes (about 4.24 GiB); source files currently exist and match their metadata sizes. GNU `cp --reflink=auto` creates independent copies, falling back to byte copies where necessary. `--prepare-only` checks and copies without running Mato or Jaiba. Existing subset metadata or copies that differ are refused, not overwritten. The database and Picoto output in `massive-100/` are isolated from both the production flock and the older 20-file subset.

`massive-100.toml` runs validation, ingestion, Wikidata taxon resolution, Picoto, registration and positive-mode annotation. The script then runs `jaiba enrich` on **current full-InChIKey candidate hits** in the same subset database. Annotation can persist successful per-file hits but exit nonzero for other files (for example, absent LOTUS biosources). In that case, the script still runs Jaiba if this dataset has current hits, but preserves Mato's nonzero exit; it skips Jaiba if there are no current hits. Jaiba errors also leave the script nonzero. Both lookups need network access, and Jaiba may need many batches; restarting reuses saved taxon and chemical lookups. Do not treat candidates as confirmed compounds. A direct `mato run --config massive-100.toml` runs only the six configured stages; the script supplies the separate Jaiba pass.

Inspect saved results without restarting either lookup or copying files. The existing inspector keeps its legacy filename and needs `sqlite3` and `xan` on `PATH`; pass the new config explicitly:

```sh
python3 /home/allardpm/git_repos/oolonek/mato/scripts/inspect_massive_20.py \
  --config /home/allardpm/git_repos/oolonek/mato/config/massive-100.toml
```

The read-only report shows persisted counts, per-file outcomes, distinct current candidate full-InChIKeys by Jaiba lookup status, the best-scoring library candidate per specimen, and a representative Picoto audit/MS2 artifact. It reports pending chemical keys if Jaiba stopped partway through. Errors from taxa absent in the frozen LOTUS CSV remain visible and keep the pipeline status nonzero; resetting the database cannot add reference data.

The inspector shows up to 20 best specimen–chemical pairs with separate **Candidate WD** and **Source WD** columns and taxonomic scores. Species QIDs come from the database. A single local LOTUS CSV scan finds both the candidate's **exact full InChIKey** Wikidata item and the structures associated with its **matched LOTUS biosource** through the 14-character InChIKey connectivity block. `exact-key` means that biosource has a record for the candidate's full key; `2D-only` means only the connectivity block matches. One biosource can list multiple structures, all displayed. In a supporting terminal, QIDs are OSC 8 hyperlinks (open or Ctrl-click); redirected output shows full URLs. Missing/ambiguous mappings are labeled, never invented. No network lookup or confirmed chemical identification is implied.

In the previous **20-file** rehearsal, the library candidate for *Petrorhagia prolifera* had full key `YGSIRXHFAUFUEJ-UHFFFAOYSA-N` ([Q104667691](https://www.wikidata.org/wiki/Q104667691)); its 9-point **species-level lineage bonus** came from a LOTUS record for the *different full key* `YGSIRXHFAUFUEJ-GPTQDWHKSA-N` ([Dianthoside, Q104402222](https://www.wikidata.org/wiki/Q104402222)) in *P. prolifera*. The first 14 characters match, but Q104667691 itself is **not** thereby shown to occur in *P. prolifera*. The score is a rank-based reranking bonus (`species` 9; genus 8), not an identification confidence.

The candidate-pair score table shows each specimen's best persisted library hit, matched LOTUS biosource, deepest shared taxonomic rank, **spectral score**, **taxonomic score**, and their **combined score**. Each `xan view` preview is preceded by a pasteable `Full table:` command; the pair-table command exposes **all** recorded hits, not only the representative row per specimen. Full-view commands use `xan`'s pager; press `q` to exit.

For an ad hoc query of the recorded annotation gaps:

```sh
sqlite3 -readonly -header -csv /media/nvme8tb/isla-gabriela/mato/massive-100/flock.sqlite \
  "SELECT taxon_name, taxon_qid, substr(error,1,100) AS error
   FROM annotation_current_runs WHERE status='error' ORDER BY taxon_name" |
  xan view --hide-index
```

`config/massive.toml` targets the much larger MassIVE dataset and the existing production flock database. Its stage list is operator-maintained; review the stages and production paths **before launching** it. Omit already completed stages in flock order when resuming instead of repeating their work. Mato does not fetch or verify the entire sapayoa inventory, download reference libraries, start SIRIUS, or annotate negative-mode spectra. Its `picoto` stage runs `--mode both` over the raw tree and **replaces matching Picoto outputs after successful per-file processing**, including previously generated paired MGF.

## Config contract

`[pipeline].stages` is an ordered, nonempty subset of `validate`, `ingest`, `enrich`, `picoto`, `register`, `annotate`, `chemical_enrich`. Omit completed stages to resume without repeating Wikidata lookups or Picoto processing; maintain the stated order. For example, after successful taxon enrichment, change the list to `["picoto", "register", "annotate"]`. To check only metadata, use `["validate"]`. Selected commands still require their inputs to exist: `register` requires matching `_ms2.mgf`, `_features.tsv` (the feature audit, not `_feature_list.tsv`) and `_sirius.mgf` for **every** mzML/mzXML under the configured raw tree. Mato checks for all three outputs before writing any registration.

`chemical_enrich` is optional and runs `jaiba enrich --db <database>` after `annotate`. It reads current annotation hits by **full** library InChIKey, reuses previous lookup results and stores possibly multiple Wikidata chemical items per key; a candidate match does not establish chemical presence in the sample. It needs hocofaisan's annotation tables in the existing database but does not need the annotation files on disk when selected alone: use `stages = ["chemical_enrich"]` to resume separately after annotation, including after an annotation run with recorded per-file errors. Use the jaiba CLI directly for non-default batch size, lookup limit or refresh.

`flock_root` and absolute/relative paths for the dataset root, database, Picoto output, and annotation references resolve relative to the **TOML file's directory**, not the process working directory. `dataset.metadata` and `dataset.ms_dir` resolve inside the dataset root. The dataset DOI is a provenance label supplied by the operator. Paths outside the dataset root for metadata or raw spectra are rejected. The required annotation files are checked before any stage runs if `annotate` is selected.

`[dataset].taxon_column` is required even when `ingest` is not selected. Set it to the **exact metadata TSV header** containing the taxon names (for example, `source_taxon` in the toy dataset or `taxon_scientific_name` in MassIVE). Mato rejects an absent or blank selection before running stages, then passes this **same TOML file** to `peregrino ingest --config`; Peregrino rejects a header not present in the metadata before changing the database. There is no automatic taxon-column selection or separate Peregrino mapping. Keep the pipeline TOML outside the source dataset unless you deliberately choose a path there.

Mato uses `cargo build --release` for the selected species, then invokes their release binaries. Cargo must be available; `CARGO` and `CARGO_TARGET_DIR` are honored if set. Reusing a config reuses Cargo's build cache. Stages remain ordered because later stages depend on earlier results; independent work *within* a stage can run concurrently. Each stage announces its species with an emoji, name and brief description. During registration the progress bar shows the latest completed raw filename; `--verbose` prints dispatched paths and forwards to cunaguaro for per-name results. Picoto prints its own per-file results. The underlying programs still print progress and summaries. See the [flock emoji key](../flock-roadmap/ROADMAP.md#flock-emoji); peregrino and picoto both use 🦈 because both are sharks.

In `config/massive.toml`, `[peregrino] jobs = 4` bounds concurrent per-file registrations (ingestion remains one SQLite transaction), `[picoto] jobs = 25` bounds Picoto file workers, and `[cunaguaro] jobs = 2` bounds concurrent taxonomy lookups (limited to 1–3 with shared Wikidata request pacing). Mato launches Hocofaisan as a child process, which annotates one file at a time; spectral-matcher parallelizes scoring **within each file** using its native Rayon pool. Set `RAYON_NUM_THREADS` on the Mato command to cap the workers inherited by Hocofaisan (for example, `RAYON_NUM_THREADS=16`); 30 or the machine maximum is not guaranteed faster. Compare elapsed time or files per minute for representative files before choosing a value; do not start competing annotation processes against the same SQLite database to simulate file-level concurrency. Missing optional `[peregrino]` and `[cunaguaro]` sections default to one worker.

During registration, concurrent peregrino processes validate and hash separate files, but their short SQLite write transactions serialize using an immediate write reservation. If registration is interrupted, `["register", "annotate"]` repeats registration without regenerating Picoto outputs; leave `picoto` in the list only if outputs need regeneration. A competing long-running database writer can still exhaust SQLite's 30-second wait.

Re-running stages delegates idempotency to the species: peregrino re-ingests a dataset transactionally, cunaguaro reuses stored successful QIDs and retries missing/failed taxon resolutions (run cunaguaro directly with `--refresh` to recheck successes), Picoto rewrites selected outputs, registration validates and replaces per-raw references, hocofaisan reuses unchanged successful annotation runs, and jaiba reuses completed chemical lookups. There is **no pipeline checkpoint or automatic retry**. If a stage fails, inspect its error and edit `stages` only after confirming the required earlier state; in particular, do not skip registration after regenerating MS2 files.

Do not start a second enrichment against the same SQLite database while a pipeline is running. An already-running cunaguaro process cannot pick up a newly built binary or changed TOML settings. Let the current run finish unless you intentionally want to stop it; restarting `enrich` skips saved successful QIDs, retries saved errors and processes taxa without a saved result. Enrichment remains database-wide, including taxa from other datasets in the same SQLite file.

Hocofaisan uses a positive-mode library only. It skips MassIVE rows with `ionization_mode = "negative"`, but unresolved taxa, absent LOTUS taxa, missing registrations, empty/invalid query MGF or other per-file annotation errors are recorded in the SQLite database and make its command fail after processing eligible files. Mato propagates that failure; previously committed per-file results remain available through peregrino's `show annotation_runs` and `show annotation_current_runs` commands. The same database may contain other datasets: hocofaisan currently processes all eligible ingested rows, not only the dataset configured in Mato.

Run `cargo test --manifest-path mato/Cargo.toml` for config and CLI regression checks. A successful metadata validation does not prove every downloaded byte matches its sapayoa receipt; verify the complete fetch separately when needed.
