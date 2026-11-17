# mato

Mato coordinates the [Isla Gabriola flock](https://github.com/oolonek/isla-gabriola-flock/blob/main/README.md), connecting research samples, mass-spectrometry measurements, and possible chemical matches into a traceable knowledge graph. One `mato run --config JOB.toml [--topN N]` prepares configured inputs and missing shared indexes, runs the selected stages, exports the graph and validates it with SHACL.

The name Mato refers to a lizard: the giant ameiva, *[Ameiva atrigularis](https://www.wikidata.org/wiki/Q2842576)* ([Catalogue of Life: 66B76](https://www.catalogueoflife.org/data/taxon/66B76)).

[![Giant ameiva (Ameiva atrigularis) photographed in Tobago](https://upload.wikimedia.org/wikipedia/commons/thumb/e/ef/Giant_ameiva_%28Ameiva_atrigularis%29.jpg/960px-Giant_ameiva_%28Ameiva_atrigularis%29.jpg)](https://commons.wikimedia.org/wiki/File:Giant_ameiva_(Ameiva_atrigularis).jpg)

Photo: [Charles J. Sharp](https://commons.wikimedia.org/wiki/File:Giant_ameiva_(Ameiva_atrigularis).jpg), [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/); image displayed without modifications.

## In the flock

The same runner serves the six-spectrum toy release and prepared datasets such as MassIVE. A job TOML selects input preparation, stage order, isolated output paths and whether a graph may contain incomplete annotations; SHACL conformity does not imply every spectrum was annotated.

## Start here

New to the project? Start with the [plain-language flock overview](https://github.com/oolonek/isla-gabriola-flock/blob/main/README.md). Copy [the toy config](config/toy.toml) to `config/toy.local.toml`, set `workspace`, then run `mato run --config config/toy.local.toml`. For a prepared MassIVE dataset, copy [the MassIVE config](config/massive.toml) to `config/massive.local.toml`, set its workspace and run `mato run --config config/massive.local.toml --topN 25`: its `[selection]` first requires reported organ and collector, sorts by filename, then selects up to 25 source records. Without `--topN`, the same config runs the unfiltered dataset. See the [technical guide](TECHNICAL.md#example-first-25-massive-records-with-reported-organs-and-collectors) for Cargo commands, selection snapshots and partial-run behavior. Rust and Cargo are required; Just and Python are not.

## Serve graphs in one QLever UI

After `mato run` prints an N-Quads graph path, use `mato qlever add --config JOB.toml --graph GRAPH.nq --name toy --port 7043` to build its QLever index and register it in the shared UI. Repeat with another name, graph and unused API port to see both backends in the **same** UI; they remain separate queryable graphs, not a merged index. On the server, `mato qlever access --config JOB.toml --ssh user@reachable-host` prints the single SSH command to run on a laptop, forwarding the UI and all registered APIs. See [the technical guide](TECHNICAL.md#serve-exported-graphs-with-qlever) for Docker prerequisites, examples and privacy limits.

To discard a job's database and Picoto output only, run `mato reset --config JOB.toml [--topN N] --yes`; this leaves its knowledge graph and all QLever data untouched. Add the **opt-in** `--kg` flag (`mato reset --config JOB.toml [--topN N] --kg --yes`) to also permanently delete that selected job's graph directory. For registered graphs inside it, Mato removes their matching QLever serving containers, index directories, and backend entries (including examples) from the shared UI. Other jobs' backends, index trees, and the shared UI SQLite database are retained. Use the same `--topN N` as the job you intend to discard; stop active jobs first. A mismatched graph path, index receipt, container image/mount/port, or UI identity makes reset refuse deletion; if a removal fails halfway, fix the cause and retry the same command. Never point two jobs at one graph directory, database, or Picoto directory.

From `mato/`, run `cargo build --release` after updating source before using `./target/release/mato`; an “unexpected argument `--kg`” means that binary predates this option. Run `./target/release/mato inspect --config JOB.toml [--topN N]` first to see the exact DB, Picoto and KG paths.

## Learn more

The [technical guide](TECHNICAL.md) covers jobs, results, resuming and configuration. The [public flock overview](https://github.com/oolonek/isla-gabriola-flock/blob/main/README.md#who-does-what) shows the other species and their current roles.

The code is available under either the [MIT license](LICENSE-MIT) or the [Apache 2.0 license](LICENSE-APACHE).

*Naming note: For new species names, we take inspiration from [colonizer](https://crates.io/crates/colonizer)'s `inspire` mode, which suggests names from the Catalogue of Life, rather than inventing each crate name ourselves.*
