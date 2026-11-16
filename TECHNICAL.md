# Mato technical guide

[Read the plain-language overview](README.md).

## Run a job

Work from the `mato` directory. The checked-in `config/toy.toml`, `config/massive.toml`, and `config/wikidata.toml` are **portable examples**, not configured jobs. Copy them once to ignored `.local.toml` files, choose paths on your own computer, and keep the database, Picoto output, shared indexes, and annotation references outside the downloaded dataset. Do not overwrite existing `.local.toml` files when updating the examples. Set `wikidata_config = "wikidata.local.toml"` at the top level of each copied job so both jobs share the same local index paths:

```sh
cd /path/to/oolonek/isla-gabriola-flock/mato
test -e config/wikidata.local.toml || cp config/wikidata.toml config/wikidata.local.toml
test -e config/toy.local.toml || cp config/toy.toml config/toy.local.toml
test -e config/massive.local.toml || cp config/massive.toml config/massive.local.toml
```

Choose both shared index destinations **once** in `config/wikidata.local.toml` (the toy and MassIVE local jobs, including MassIVE top-N runs, use that file):

```toml
[indexes]
taxon_index = "/path/to/shared/wikidata/taxa.sqlite"
chemical_index = "/path/to/shared/wikidata/chemicals.sqlite"
```

Edit every `/path/to/` value in the copied files and set the `wikidata_config` override in each local job before building and inspecting:

```sh
cargo build --release
./target/release/mato inspect --config config/toy.local.toml
```

After inspecting your selected paths, refresh both snapshots once before running jobs; subsequent jobs reuse them. This fetch can require substantial network time and disk space:

```sh
./target/release/mato indexes refresh --config config/wikidata.local.toml
./target/release/mato run --config config/toy.local.toml
```

To refresh just one snapshot later, add `--only taxon` or `--only chemical` to the `indexes refresh --config config/wikidata.local.toml` command. Do not refresh an index while another process writes it. The toy pipeline includes `chemical_enrich` after annotation.

Replace `config/toy.local.toml` with any configured job file. `inspect` prints the dataset, DOI, selected stages, resolved database and Picoto paths, whether those outputs exist, and stored dataset IDs and row counts if the database exists. It reads SQLite without creating or changing it. Check the paths **before** resetting or running anything. An independent job needs its own database and Picoto output directory outside its source dataset. The supplied `config/massive.toml` example selects only `annotate` without `--topN`; `--topN 250` selects an isolated pair instead. Set the copied local job's paths first.

`annotation_runs` is a historical **attempt** count: a failed file retried twice has two rows. `annotation_current_runs` counts the latest attempt per registered file, so compare that with `spectrum_files` rather than expecting one annotation row per sample; files skipped before annotation have no attempt.

To start the **toy** job fresh, stop all Mato/species processes using its database or Picoto output. Restore `[pipeline].stages` in `config/toy.local.toml` to the full sequence needed to build the results (for example, `["validate", "ingest", "enrich", "picoto", "register", "annotate", "chemical_enrich"]`), then inspect the config again. Only when deliberately discarding a previous job, delete that configured database, SQLite sidecars, and Picoto output directory **permanently** with explicit confirmation (for MassIVE top-N, use the separate procedure below instead):

```sh
./target/release/mato reset --config config/toy.local.toml --yes
./target/release/mato run --config config/toy.local.toml
```

`reset` refuses paths inside the input dataset or source tree and an output directory that contains either. **It cannot know whether other jobs share the configured database or output directory.** Do not reset `config/massive.local.toml` without `--topN` or another shared job. `reset --config config/massive.local.toml --topN 250 --yes` is only for deliberately deleting that isolated N's DB and Picoto output; it does not delete production outputs or the old top-N database. Running a resume-only stage list such as `["annotate"]` against a freshly reset database cannot work; include `ingest` and required earlier stages.

To **resume**, do not reset. Keep the database and Picoto files, edit the selected stage list to begin at the stage needing retry, and keep later dependent stages in order. For example, after successful registration and partial annotation in a MassIVE top-N run, use `top_n_stages = ["annotate", "chemical_enrich"]` with the **same `--topN 250`** on each command: unchanged successes and completed chemical lookups are reused, while failed annotation files are retried. After regenerating Picoto files, rerun `register`. There is no automatic checkpoint. For a full-stage rerun without deleting data, restore the full `top_n_stages` list in the same TOML and keep `--topN 250`.

Inspect any existing table or view without typing a database path:

```sh
./target/release/mato show annotation_current_runs --config config/toy.local.toml --limit 20
./target/release/mato show annotation_runs --config config/toy.local.toml --limit 200
```

`show` uses Peregrino's read-only table preview. If the DB is missing, it reports that explicitly rather than returning SQLite's “unable to open database” error; run the configured pipeline with `ingest` selected before inspecting results. Add `--verbose` to `run` for dispatched filenames and Cunaguaro per-name output.

## Run reports

Each `mato run` with a valid job config writes a new Markdown report in `reports/` beside the selected SQLite file (for `--topN 250`, `<top_n_root>/250/reports/run-<timestamp>-<pid>-<sequence>.md`). Mato prints the exact report path at the end, even for a partial or failed pipeline. Reports are atomic, are never overwritten, and survive `mato reset` so you can compare attempts. If the report itself cannot be written, Mato exits nonzero; an earlier pipeline error remains the primary error. A force-killed process cannot finalize a report.

The report records `complete`, `partial` (per-file annotation errors; later stages used saved hits), or `failed` (fatal stage error); selected stages and row limit (`250` for `--topN 250`, no limit when omitted); dataset DOI, input and selected output locations; dataset-specific counts of ingested samples, spectra, registrations, current annotation hit files and hits; and a database-wide chemical lookup count. The row limit describes the first N metadata rows selected and their referenced raw files, **not** how many records this invocation added. Counts are **database state at finish**, not counts of new records in this invocation. An unavailable DB or table is shown as **not available**, not zero. The failed-file section lists the latest outstanding annotation error per file for this dataset, including taxon/QID/reason and whether the error was recorded during this run; new errors from other datasets in a shared DB are included with their dataset ID. A report can still be generated if the DB cannot be read, with the inspection failure shown instead of invented metrics. Copy the report's `inspect` and `show` commands to inspect the live DB; the saved report is a snapshot, while the database may change on retries or reset.
`Outstanding failed files` counts only this dataset's latest unresolved annotation errors; the table may additionally show errors newly recorded for other datasets in a shared database.

## Example: download the Zenodo toy dataset and export a KG

This walkthrough starts from the checked-in [toy job template](config/toy.toml) and [shared-index template](config/wikidata.toml). Copy them to ignored `config/toy.local.toml` and `config/wikidata.local.toml` as explained above. Choose an external dataset directory, separate job database and Picoto output, annotation reference directory, and two shared index locations **on your own computer**. Set `wikidata_config = "wikidata.local.toml"` at the top level of the copied toy job and make its paths agree with the `DATA`, `REF`, and `DB` variables below. Use a new DB/Picoto pair for each independent job; never point two jobs at the same outputs or mistake a shared index for a job DB. Run these steps in order in **one shell**; stop on any failed command.

1. From the flock directory containing the sibling crates, fetch the public [Zenodo ENPKG toy DOI](https://doi.org/10.5281/zenodo.10018264) with Sapayoa. The concept DOI currently resolves to record `10018590`; if it resolves to a different version, choose a version-specific DOI and matching data root rather than mixing releases. Sapayoa saves the published ZIP without extracting it. The public Zenodo download needs no account; ensure your chosen external volume is mounted and writable, and do not put credentials in command lines or committed TOML.

   ```bash
   ROOT="$(pwd)"  # run this step from the isla-gabriola-flock directory
   DATA=/path/to/extracted/zenodo-10018590
   REF=/path/to/references
   DB=/path/to/job/toy.sqlite
   cd "$ROOT"
   cargo run --release --manifest-path sapayoa/Cargo.toml -- \
     fetch 10.5281/zenodo.10018264 "$DATA" --all
   ```

2. Extract the single downloaded ZIP **outside the crate**, into the dataset root named in `toy.local.toml`. The published archive has `metadata/` and `msdata/` at its top level. `unzip -n` does not overwrite previously extracted files; if extraction was interrupted, inspect the existing tree and archive before continuing rather than assuming skipped files are valid.

   ```bash
   shopt -s nullglob
   archives=( "$DATA"/*.zip )
   if (( ${#archives[@]} != 1 )); then
     printf 'Expected one Zenodo ZIP in %s; inspect the fetch first\n' "$DATA" >&2
     exit 1  # stop here; do not continue with an old extracted tree
   else
     unzip -n "${archives[0]}" -d "$DATA"
   fi
   test -f "$DATA/metadata/metadata.tsv" &&
     test -d "$DATA/msdata/raw"
   ```

3. Download the **positive-mode** ISDB LOTUS MGF and frozen LOTUS metadata into the paths configured under `[annotation]` in `toy.local.toml`. The MGF is about 939 MB; allow time and disk space. Verify the published MD5 checksums before using either input; `gzip -dk` leaves the downloaded compressed original intact and refuses to replace an existing CSV.

   ```bash
   mkdir -p "$REF"
   curl --fail --location --show-error \
     --output "$REF/isdb_lotus_pos_energySum.mgf" \
     'https://zenodo.org/records/14887271/files/isdb_lotus_pos_energySum.mgf?download=1'
   curl --fail --location --show-error \
     --output "$REF/230106_frozen_metadata.csv.gz" \
     'https://zenodo.org/records/7534071/files/230106_frozen_metadata.csv.gz?download=1'
   printf '%s  %s\n' c025e3365ca913b46ec4f6637d381911 \
     "$REF/isdb_lotus_pos_energySum.mgf" | md5sum -c -
   printf '%s  %s\n' 324579ae4a4785bf1b50b532b0a00eec \
     "$REF/230106_frozen_metadata.csv.gz" | md5sum -c -
   gzip -dk "$REF/230106_frozen_metadata.csv.gz"
   ```

4. From `mato/`, build once and prepare the **shared** taxon and chemical Wikidata snapshots. Set the two index destinations in your ignored `config/wikidata.local.toml` before refreshing, then keep that same file selected in the toy job. This full QLever download/index build can take substantial network time, disk and memory; refresh once, reuse across jobs, and never refresh concurrently with a writer. No private Wikidata credentials are needed for the documented public QLever endpoint.

   ```bash
   cd "$ROOT/mato"
   cargo build --release
   ./target/release/mato indexes refresh --config config/wikidata.local.toml
   ```

5. Inspect the resolved input, DB, Picoto and index paths **before** running; the toy job selects `validate`, `ingest`, `enrich`, `picoto`, `register`, `annotate`, and `chemical_enrich` in that order. Run it and inspect the recorded annotation outcomes. The input has three metadata rows and six mzXML files; Cunaguaro resolves *Datura stramonium*, while QC/blank rows and negative-mode spectra remain ingested but are skipped by positive-only Hocofaisan. Mato writes a run report beside the job DB. A nonzero partial annotation status may still leave saved hits and chemical lookups: review `annotation_current_runs`, `annotation_runs`, and the report before exporting, rather than treating partial results as a complete run.

   ```bash
   ./target/release/mato inspect --config config/toy.local.toml
   ./target/release/mato run --config config/toy.local.toml
   ./target/release/mato show annotation_current_runs --config config/toy.local.toml --limit 20
   ./target/release/mato show annotation_runs --config config/toy.local.toml --limit 20
   ./target/release/mato show chemical_lookups --config config/toy.local.toml --limit 20
   ```

6. Discover the **dataset ID in this job DB**; do not assume it is `1` or reuse an ID from another job. In the `datasets` output, match the row for DOI `10.5281/zenodo.10018264`. Enter that row's numeric `id` at the prompt below. Peregrino exports an actual N-Quads graph from the registered spectra and database; it refuses to overwrite an existing graph, so make a fresh output directory for each export. `kg export` checks registered file content; fix missing/changed files rather than bypassing its checks.

   ```bash
   ./target/release/mato show datasets --config config/toy.local.toml
   read -r -p 'Dataset id for DOI 10.5281/zenodo.10018264: ' DATASET_ID
   [[ "$DATASET_ID" =~ ^[0-9]+$ ]] || { printf 'Expected a numeric dataset id\n' >&2; exit 1; }
   WORK="$(mktemp -d "$HOME/mato-kg-toy.XXXXXX")"
   cargo run --release --manifest-path ../peregrino/Cargo.toml -- kg export \
     --db "$DB" --dataset-id "$DATASET_ID" --out "$WORK/graph.nq"
   ```

7. Optionally validate the exported N-Quads against Peregrino's bundled SHACL shapes with its Rust `kg validate` command; it exits nonzero on violations. Then run an example SPARQL `SELECT` against the **same graph**. Validation and `kg query` load the graph in memory, so use a bounded dataset such as this toy; for larger exports, see [Peregrino's QLever setup](https://github.com/oolonek/peregrino/blob/master/kg/QLEVER.md).

   ```bash
   cargo run --release --manifest-path ../peregrino/Cargo.toml -- kg validate --graph "$WORK/graph.nq"
   cargo run --release --manifest-path ../peregrino/Cargo.toml -- kg query \
     --graph "$WORK/graph.nq" --query ../peregrino/kg/taxonomic-evidence.rq
   ```

The graph links local sample records, acquisitions, extracted features and scored candidate hits to resolved external identifiers; a candidate chemical match is **not** proof of a metabolite's presence. See [Peregrino's KG semantics](https://github.com/oolonek/peregrino/blob/master/TECHNICAL.md#wikidata-linked-local-knowledge-graph) for graph interpretation. Do not use `mato reset` for a first run or while any other job could share its paths; to resume a partial run, retain the DB/Picoto outputs and adjust only the selected stage list as explained above.

## Example: first 250 MassIVE metadata rows

The [MassIVE job template](config/massive.toml) selects only `annotate` without `--topN`; its database and Picoto paths must be set deliberately for an existing production job. Copy it to ignored `config/massive.local.toml`, choose paths on your own machine, and set `wikidata_config = "wikidata.local.toml"` at the top level. With a positive `--topN 250`, Mato instead selects the configured full `top_n_stages` (`validate`, `ingest`, `enrich`, `picoto`, `register`, `annotate`, `chemical_enrich`) and derives an isolated database at `<top_n_root>/250/flock.sqlite` and Picoto output at `<top_n_root>/250/picoto`. Choose a separate `top_n_root` for this rehearsal. Inspect before running, from the `mato` directory after building Mato:

```sh
./target/release/mato inspect --config config/massive.local.toml --topN 250
./target/release/mato run --config config/massive.local.toml --topN 250
./target/release/mato show annotation_current_runs --config config/massive.local.toml --topN 250 --limit 200
```

`--topN 250` selects the first 250 metadata rows in source order and their referenced raw files, not a curated taxon or polarity; negative-mode files are ingested but skipped by positive-only annotation. If `inspect` reports the isolated DB as `(missing)`, `show` cannot read it until `ingest` succeeds. Do **not** reset automatically on a new run or a resume. To rerun every stage on the same isolated data, restore the full `top_n_stages` list in `config/massive.local.toml` and run again with `--topN 250`; to resume, shorten only `top_n_stages` as described above and keep `--topN 250`. Neither action changes the production stage list `[pipeline].stages`.

Only when intentionally discarding this 250-row job: stop processes using its DB/Picoto output, inspect and confirm the selected paths, then explicitly run:

```sh
./target/release/mato reset --config config/massive.local.toml --topN 250 --yes
```

This deletes only `<top_n_root>/250/flock.sqlite` (and its SQLite sidecars) and `<top_n_root>/250/picoto`; it does **not** delete the ordinary job's configured database or Picoto output. Do not point another job at these selected paths: Mato cannot detect sharing, and resetting a shared pair would destroy another job's outputs. Mato does not fetch source data or reference libraries, start SIRIUS, or annotate negative-mode spectra. Without `--topN`, Picoto processes the configured raw tree and replaces matching outputs after successful per-file processing.

## Config contract

`[pipeline].stages` is an ordered, nonempty subset of `validate`, `ingest`, `enrich`, `picoto`, `register`, `annotate`, `chemical_enrich`; it controls ordinary runs, including MassIVE's current annotation-only production run. For MassIVE `--topN N`, `[pipeline].top_n_stages` takes its place and `[database].top_n_root` supplies the isolated output parent. Both settings and a positive N are required for top-N mode. Edit `top_n_stages` (not `stages`) to resume or rerun the top-N job. Omit completed stages to resume without repeating processing; taxon `enrich` reuses matching results from the current shared index generation. Maintain stage order. For example, after successful taxon enrichment, change the selected list to `["picoto", "register", "annotate"]`. To check only metadata, use `["validate"]`. Selected commands still require their inputs to exist: `register` requires matching `_ms2.mgf`, `_features.tsv` (the feature audit, not `_feature_list.tsv`) and `_sirius.mgf` for every selected mzML/mzXML. Without `--topN`, selection includes every supported raw file under the configured raw tree; with `--topN N`, it selects the first N metadata rows and their referenced raw files. Mato checks for all three outputs before writing any registration.

`enrich` uses the shared taxon index selected by the job's `wikidata_config`; `chemical_enrich` uses its chemical index. Both paths resolve relative to that shared file when not absolute. `mato inspect --config CONFIG` reports their paths and presence. The checked-in `toy.toml` and `massive.toml` are templates; set `wikidata_config = "wikidata.local.toml"` in each ignored local copy. Refresh both with `./target/release/mato indexes refresh --config config/wikidata.local.toml`, or add `--only taxon` or `--only chemical` to refresh one. `mato reset` leaves both shared indexes untouched.

`chemical_enrich` is optional and runs `cunaguaro chemical-enrich --db <database>` after `annotate`. It reads current annotation hits by **full** library InChIKey, reuses previous lookup results and stores possibly multiple Wikidata chemical items per key; a candidate match does not establish chemical presence in the sample. It needs hocofaisan's annotation tables in the existing database but does not need the annotation files on disk when selected alone: use `stages = ["chemical_enrich"]` for an ordinary run or `top_n_stages = ["chemical_enrich"]` with the same `--topN N` to resume after top-N annotation, including after an annotation run with recorded per-file errors. Use the Cunaguaro CLI directly for non-default batch size, lookup limit or refresh. `cunaguaro propose --db <database>` prints the graph proposal and logs identities needing review.

To avoid per-key WDQS requests in `chemical_enrich`, configure the shared chemical snapshot once under `[indexes].chemical_index` in your `config/wikidata.local.toml`, then refresh it with `./target/release/mato indexes refresh --config config/wikidata.local.toml --only chemical`. Mato passes the shared file read-only to Cunaguaro and reports whether it is present in `inspect`; a missing configured index fails before starting stages. `mato reset` deletes only the selected job's DB/Picoto outputs, not the shared index. Refresh explicitly when needed (for example weekly), not concurrently with writers. See [Cunaguaro](https://github.com/oolonek/cunaguaro/blob/master/TECHNICAL.md#bulk-qlever-chemical-index) for the advanced standalone CLI and offline `--from-db` option.

`flock_root` and absolute/relative paths for the dataset root, database, Picoto output, annotation references and `top_n_root` resolve relative to the **job TOML file's directory**, not the process working directory. With `--topN N`, the DB and Picoto paths derive from `top_n_root/N/` instead of the normal `[database]` and `[picoto]` output paths. The optional `wikidata_config` job setting resolves relative to that directory; otherwise Mato uses its shipped `config/wikidata.toml`. Index paths inside the shared file resolve relative to **its** directory unless absolute. `dataset.metadata` and `dataset.ms_dir` resolve inside the dataset root. The dataset DOI is a provenance label supplied by the operator. Paths outside the dataset root for metadata or raw spectra are rejected. The required annotation files are checked before any stage runs if `annotate` is selected.

`[dataset].taxon_column` is required even when `ingest` is not selected. Set it to the **exact metadata TSV header** containing the taxon names (for example, `source_taxon` in the toy dataset or `taxon_scientific_name` in MassIVE). Mato rejects an absent or blank selection before running stages, then passes this **same TOML file** to `peregrino ingest --config`; Peregrino rejects a header not present in the metadata before changing the database. There is no automatic taxon-column selection or separate Peregrino mapping. Keep the pipeline TOML outside the source dataset unless you deliberately choose a path there.

Mato uses `cargo build --release` for the selected species, then invokes their release binaries. Cargo must be available; `CARGO` and `CARGO_TARGET_DIR` are honored if set. Reusing a config reuses Cargo's build cache. Stages remain ordered because later stages depend on earlier results; independent work *within* a stage can run concurrently. Each stage announces its species with an emoji, name and brief description. During registration the progress bar shows the latest completed raw filename; `--verbose` prints dispatched paths and forwards to cunaguaro for per-name results. Picoto prints its own per-file results. The underlying programs still print progress and summaries. Peregrino and Picoto both use 🦈 because both are sharks.

In `config/massive.toml`, `[peregrino] jobs = 4` bounds concurrent per-file registrations (ingestion remains one SQLite transaction), and `[picoto] jobs = 25` bounds Picoto file workers. Taxon enrichment reads the shared local index rather than pacing per-name Wikidata requests. Mato launches Hocofaisan as a child process, which annotates one file at a time; spectral-matcher parallelizes scoring **within each file** using its native Rayon pool. Set `RAYON_NUM_THREADS` on the Mato command to cap the workers inherited by Hocofaisan (for example, `RAYON_NUM_THREADS=16`); 30 or the machine maximum is not guaranteed faster. Compare elapsed time or files per minute for representative files before choosing a value; do not start competing annotation processes against the same SQLite database to simulate file-level concurrency. Missing optional `[peregrino]` defaults to one worker.

During registration, concurrent peregrino processes validate and hash separate files, but their short SQLite write transactions serialize using an immediate write reservation. If registration is interrupted, `["register", "annotate"]` repeats registration without regenerating Picoto outputs; leave `picoto` in the list only if outputs need regeneration. A competing long-running database writer can still exhaust SQLite's 30-second wait.

Re-running stages delegates idempotency to the species: peregrino re-ingests a dataset transactionally, cunaguaro reuses stored successful QIDs and retries missing/failed taxon resolutions (run cunaguaro directly with `--refresh` to recheck successes), Picoto rewrites selected outputs, registration validates and replaces per-raw references, hocofaisan reuses unchanged successful annotation runs, and cunaguaro chemical enrichment reuses completed lookups. There is **no pipeline checkpoint or automatic retry**. If a stage fails, inspect its error and edit the selected stage list (`top_n_stages` with `--topN`, otherwise `stages`) only after confirming the required earlier state; in particular, do not skip registration after regenerating MS2 files.

Do not start a second enrichment against the same SQLite database while a pipeline is running. An already-running cunaguaro process cannot pick up a newly built binary or changed TOML settings. Let the current run finish unless you intentionally want to stop it; restarting `enrich` skips saved successful QIDs, retries saved errors and processes taxa without a saved result. Enrichment remains database-wide, including taxa from other datasets in the same SQLite file.

Hocofaisan uses a positive-mode library only. It skips MassIVE rows with `ionization_mode = "negative"`, but unresolved taxa, absent LOTUS taxa, missing registrations, empty/invalid query MGF or other per-file annotation errors are recorded in SQLite. After completing eligible files, Hocofaisan exits `2` when any per-file errors were recorded (`1` remains a fatal stage failure). Mato treats only this `annotate` exit `2` as partial: later configured stages, including `chemical_enrich`, run on the saved current hits. Mato then exits nonzero with a reprocessing warning rather than claiming full success. The error rows remain in `annotation_runs`; successful unchanged files are reused and failed files are retried on the next `annotate` run. Other stage failures still stop immediately. Hocofaisan currently processes all eligible ingested rows in the DB, not only the dataset configured in Mato.

Run `cargo test --manifest-path mato/Cargo.toml` for config and CLI regression checks. A successful metadata validation does not prove every downloaded byte matches its sapayoa receipt; verify the complete fetch separately when needed.
