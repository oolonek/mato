# Mato technical guide

[Read the plain-language overview](README.md).

## Run a job

Work from `mato/`. The checked-in `config/toy.toml` and `config/massive.toml` are portable examples. Copy the job you need to an ignored `.local.toml`, set its single `workspace` to an **existing writable directory** outside the source checkout, and leave the relative paths under `[dataset]`, `[database]`, `[picoto]`, `[indexes]`, `[annotation]` and `[knowledge_graph]` unless you need another layout. Both jobs can name the same workspace-root `mato/wikidata/*.sqlite` snapshots; their job databases and Picoto outputs remain separate. Keep `flock_root` relative to the config file and check the actual selected paths before resetting a job.

```sh
cd /path/to/oolonek/isla-gabriola-flock/mato
test -e config/toy.local.toml || cp config/toy.toml config/toy.local.toml
# Edit only workspace = "/path/to/your/writable/workspace" in config/toy.local.toml.
cargo run --release -- bootstrap --config config/toy.local.toml
```

For the toy, `bootstrap` verifies and prepares the version-pinned public Zenodo ZIP and two reference files; checks existing shared index snapshots or builds missing ones from QLever; runs all seven stages; discovers the dataset ID by DOI and source root; exports a **new** N-Quads file; validates it with Rust SHACL; then prints the DB, graph, and example query command. Existing verified downloads and completed snapshots are reused. It does **not** erase or reset a job DB; a rerun reuses species state and produces a new graph. A corrupted or incomplete nonempty index is rejected rather than silently replaced. The command requires Rust/Cargo and network access for missing inputs/indexes. It is specific to the pinned Zenodo toy; for other data, prepare inputs and use `mato run`.

Inspect and refresh shared indexes explicitly for another configured job, without a second TOML:

```sh
./target/release/mato inspect --config config/toy.local.toml
./target/release/mato indexes refresh --config config/toy.local.toml --only taxon
./target/release/mato indexes refresh --config config/toy.local.toml --only chemical
./target/release/mato run --config config/toy.local.toml
```

The separate `indexes refresh` commands are optional for toy bootstrap. Do not refresh an index concurrently with another writer. Replace `config/toy.local.toml` with any configured job file for ordinary runs. `inspect` shows the resolved dataset, DOI, selected stages, DB, Picoto paths, index locations, and stored row counts when present; it reads SQLite without changing it. An independent job needs its own database and Picoto output outside its source dataset. The MassIVE template's ordinary stage list selects only `annotate`; `--topN 250` instead selects its full top-N stage list and isolated outputs.

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

**One editable path, one command.** From `mato/`, copy [config/toy.toml](config/toy.toml) once, set `workspace` in your ignored copy to an **existing writable directory** outside the checkout, and run:

```sh
test -e config/toy.local.toml || cp config/toy.toml config/toy.local.toml
# Edit workspace = "/path/to/your/writable/workspace" in config/toy.local.toml.
cargo run --release -- bootstrap --config config/toy.local.toml
```

In plain language, Mato checks/downloads the toy spectra and annotation references, checks/creates reusable Wikidata lookup tables, processes three metadata rows and six spectra, writes a Markdown run report and a **new** graph file, and validates that graph. The command prints the graph path and a ready-to-run SPARQL query command. It does not require `unzip`, `curl`, `sqlite3`, Python, or Just. Allow disk space and time for the reference MGF (~939 MB), LOTUS CSV, and multi-million-row Wikidata snapshots on a clean machine.

Technically, `bootstrap` pins the [version-specific Zenodo record 10018590](https://zenodo.org/records/10018590) for the toy concept DOI [10.5281/zenodo.10018264](https://doi.org/10.5281/zenodo.10018264). Sapayoa checks the published file receipt; Mato verifies every extracted ZIP entry and the published reference checksums before use. Existing identical files and complete shared indexes are reused; mismatches or incomplete nonempty snapshots stop the command rather than silently replacing data. All output paths resolve from the single configured workspace, outside the downloaded source and checkout. The graph is an N-Quads file under `[knowledge_graph].output_dir` with a unique name; Mato finds the database dataset ID by DOI and root, invokes Peregrino `kg export`, and requires Rust SHACL `kg validate` to succeed. A failed or partial pipeline stops before export. Running bootstrap again does **not** reset the job DB or Picoto results and never overwrites a previous graph.

To inspect the job or run a bounded example query, use the paths printed by bootstrap:

```sh
./target/release/mato inspect --config config/toy.local.toml
./target/release/mato show annotation_current_runs --config config/toy.local.toml --limit 20
./target/release/mato show chemical_lookups --config config/toy.local.toml --limit 20
# Paste the printed Peregrino kg query command; it includes the exact graph path.
```

The graph links local sample records, analyses, features and ranked **candidate** annotations to Wikidata taxon and chemical QIDs; a candidate does not prove a molecule is present. [Peregrino's KG semantics](https://github.com/oolonek/peregrino/blob/master/TECHNICAL.md#wikidata-linked-local-knowledge-graph) explain the joins. For graphs too large for in-memory SHACL/query, see [QLever indexing](https://github.com/oolonek/peregrino/blob/master/kg/QLEVER.md). `bootstrap` currently supports only this pinned toy; general `mato run` remains available for other prepared datasets.

[Just](https://just.systems/man/en/) is an optional command runner, not a downloader or integrity check. A `just toy` wrapper around this one command would add an installation and another file without removing any remaining setup, so no Justfile is required.

## Example: first 250 MassIVE metadata rows

The [MassIVE job template](config/massive.toml) selects only `annotate` without `--topN`; its database and Picoto paths must be set deliberately for an existing ordinary job. Copy it to ignored `config/massive.local.toml` and set the **same workspace** used in the toy job if you want to reuse the same `[indexes]` snapshots and reference files. With a positive `--topN 250`, Mato instead selects the configured full `top_n_stages` (`validate`, `ingest`, `enrich`, `picoto`, `register`, `annotate`, `chemical_enrich`) and derives an isolated database at `<top_n_root>/250/flock.sqlite` and Picoto output at `<top_n_root>/250/picoto`. Inspect before running from `mato/`:

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

This deletes only `<top_n_root>/250/flock.sqlite` (and its SQLite sidecars) and `<top_n_root>/250/picoto`; it does **not** delete the ordinary job's configured database or Picoto output. Do not point another job at these selected paths: Mato cannot detect sharing, and resetting a shared pair would destroy another job's outputs. Only toy `bootstrap` fetches public inputs; general `mato run` does not fetch MassIVE source data or references, start SIRIUS, or annotate negative-mode spectra. Without `--topN`, Picoto processes the configured raw tree and replaces matching outputs after successful per-file processing.

## Config contract

`[pipeline].stages` is an ordered, nonempty subset of `validate`, `ingest`, `enrich`, `picoto`, `register`, `annotate`, `chemical_enrich`; it controls ordinary runs, including MassIVE's current annotation-only production run. For MassIVE `--topN N`, `[pipeline].top_n_stages` takes its place and `[database].top_n_root` supplies the isolated output parent. Both settings and a positive N are required for top-N mode. Edit `top_n_stages` (not `stages`) to resume or rerun the top-N job. Omit completed stages to resume without repeating processing; taxon `enrich` reuses matching results from the current shared index generation. Maintain stage order. For example, after successful taxon enrichment, change the selected list to `["picoto", "register", "annotate"]`. To check only metadata, use `["validate"]`. Selected commands still require their inputs to exist: `register` requires matching `_ms2.mgf`, `_features.tsv` (the feature audit, not `_feature_list.tsv`) and `_sirius.mgf` for every selected mzML/mzXML. Without `--topN`, selection includes every supported raw file under the configured raw tree; with `--topN N`, it selects the first N metadata rows and their referenced raw files. Mato checks for all three outputs before writing any registration.

`enrich` uses `[indexes].taxon_index` and `chemical_enrich` uses `[indexes].chemical_index` from the **same job TOML**. Set both to persistent paths in a common workspace to share complete snapshots across jobs. `mato inspect --config CONFIG` reports their paths and presence. Refresh both with `./target/release/mato indexes refresh --config config/toy.local.toml`, or add `--only taxon` or `--only chemical` to refresh one. `mato reset` leaves both shared indexes untouched.

`chemical_enrich` is optional and runs `cunaguaro chemical-enrich --db <database>` after `annotate`. It reads current annotation hits by **full** library InChIKey, reuses previous lookup results and stores possibly multiple Wikidata chemical items per key; a candidate match does not establish chemical presence in the sample. It needs hocofaisan's annotation tables in the existing database but does not need the annotation files on disk when selected alone: use `stages = ["chemical_enrich"]` for an ordinary run or `top_n_stages = ["chemical_enrich"]` with the same `--topN N` to resume after top-N annotation, including after an annotation run with recorded per-file errors. Use the Cunaguaro CLI directly for non-default batch size, lookup limit or refresh. `cunaguaro propose --db <database>` prints the graph proposal and logs identities needing review.

To avoid per-key WDQS requests in `chemical_enrich`, keep the shared chemical snapshot at `[indexes].chemical_index` in your job config. Toy `bootstrap` builds it when missing; for other jobs, use `./target/release/mato indexes refresh --config config/massive.local.toml --only chemical`. Mato passes the shared file read-only to Cunaguaro and reports whether it is present in `inspect`; a missing configured index fails before starting stages. `mato reset` deletes only the selected job's DB/Picoto outputs, not the shared index. Refresh explicitly when needed (for example weekly), not concurrently with writers. See [Cunaguaro](https://github.com/oolonek/cunaguaro/blob/master/TECHNICAL.md#bulk-qlever-chemical-index) for the advanced standalone CLI and offline `--from-db` option.

`workspace` in the job TOML is an absolute writable root for dataset folders, job DBs, Picoto outputs, annotation references, shared indexes and graph exports; relative paths for those fields resolve beneath it, not the process directory. Absolute paths remain absolute. `flock_root` and `[annotation].settings` resolve relative to the job TOML file; `dataset.metadata` and `dataset.ms_dir` resolve inside the dataset root. With `--topN N`, the DB and Picoto paths derive from `top_n_root/N/` instead of the normal `[database]` and `[picoto]` paths. The dataset DOI is supplied by the operator. Paths outside the dataset root for metadata or raw spectra are rejected. The required annotation files are checked before any stage runs if `annotate` is selected.

`[dataset].taxon_column` is required even when `ingest` is not selected. Set it to the **exact metadata TSV header** containing the taxon names (for example, `source_taxon` in the toy dataset or `taxon_scientific_name` in MassIVE). Mato rejects an absent or blank selection before running stages, then passes this **same TOML file** to `peregrino ingest --config`; Peregrino rejects a header not present in the metadata before changing the database. There is no automatic taxon-column selection or separate Peregrino mapping. Keep the pipeline TOML outside the source dataset unless you deliberately choose a path there.

Mato uses `cargo build --release` for the selected species, then invokes their release binaries. Cargo must be available; `CARGO` and `CARGO_TARGET_DIR` are honored if set. Reusing a config reuses Cargo's build cache. Stages remain ordered because later stages depend on earlier results; independent work *within* a stage can run concurrently. Each stage announces its species with an emoji, name and brief description. During registration the progress bar shows the latest completed raw filename; `--verbose` prints dispatched paths and forwards to cunaguaro for per-name results. Picoto prints its own per-file results. The underlying programs still print progress and summaries. Peregrino and Picoto both use 🦈 because both are sharks.

In `config/massive.toml`, `[peregrino] jobs = 4` bounds concurrent per-file registrations (ingestion remains one SQLite transaction), and `[picoto] jobs = 25` bounds Picoto file workers. Taxon enrichment reads the shared local index rather than pacing per-name Wikidata requests. Mato launches Hocofaisan as a child process, which annotates one file at a time; spectral-matcher parallelizes scoring **within each file** using its native Rayon pool. Set `RAYON_NUM_THREADS` on the Mato command to cap the workers inherited by Hocofaisan (for example, `RAYON_NUM_THREADS=16`); 30 or the machine maximum is not guaranteed faster. Compare elapsed time or files per minute for representative files before choosing a value; do not start competing annotation processes against the same SQLite database to simulate file-level concurrency. Missing optional `[peregrino]` defaults to one worker.

During registration, concurrent peregrino processes validate and hash separate files, but their short SQLite write transactions serialize using an immediate write reservation. If registration is interrupted, `["register", "annotate"]` repeats registration without regenerating Picoto outputs; leave `picoto` in the list only if outputs need regeneration. A competing long-running database writer can still exhaust SQLite's 30-second wait.

Re-running stages delegates idempotency to the species: peregrino re-ingests a dataset transactionally, cunaguaro reuses stored successful QIDs and retries missing/failed taxon resolutions (run cunaguaro directly with `--refresh` to recheck successes), Picoto rewrites selected outputs, registration validates and replaces per-raw references, hocofaisan reuses unchanged successful annotation runs, and cunaguaro chemical enrichment reuses completed lookups. There is **no pipeline checkpoint or automatic retry**. If a stage fails, inspect its error and edit the selected stage list (`top_n_stages` with `--topN`, otherwise `stages`) only after confirming the required earlier state; in particular, do not skip registration after regenerating MS2 files.

Do not start a second enrichment against the same SQLite database while a pipeline is running. An already-running cunaguaro process cannot pick up a newly built binary or changed TOML settings. Let the current run finish unless you intentionally want to stop it; restarting `enrich` skips saved successful QIDs, retries saved errors and processes taxa without a saved result. Enrichment remains database-wide, including taxa from other datasets in the same SQLite file.

Hocofaisan uses a positive-mode library only. It skips MassIVE rows with `ionization_mode = "negative"`, but unresolved taxa, absent LOTUS taxa, missing registrations, empty/invalid query MGF or other per-file annotation errors are recorded in SQLite. After completing eligible files, Hocofaisan exits `2` when any per-file errors were recorded (`1` remains a fatal stage failure). Mato treats only this `annotate` exit `2` as partial: later configured stages, including `chemical_enrich`, run on the saved current hits. Mato then exits nonzero with a reprocessing warning rather than claiming full success. The error rows remain in `annotation_runs`; successful unchanged files are reused and failed files are retried on the next `annotate` run. Other stage failures still stop immediately. Hocofaisan currently processes all eligible ingested rows in the DB, not only the dataset configured in Mato.

Run `cargo test --manifest-path mato/Cargo.toml` for config and CLI regressions. `mato bootstrap` additionally verifies the version-pinned Zenodo toy download, extraction and reference checksums at runtime; metadata validation alone does not prove downloaded-byte integrity.
