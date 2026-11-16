# mato

Mato coordinates the [Isla Gabriola flock](https://github.com/oolonek/isla-gabriola-flock/blob/main/README.md), a group of tools that connects research samples, mass-spectrometry measurements, and possible chemical matches into a traceable record that can become a searchable knowledge graph. Its `bootstrap` command downloads and verifies the toy data and reference files, runs the complete pipeline, and exports and validates a KG.

The name Mato refers to a lizard: the giant ameiva, *[Ameiva atrigularis](https://www.wikidata.org/wiki/Q2842576)* ([Catalogue of Life: 66B76](https://www.catalogueoflife.org/data/taxon/66B76)).

[![Giant ameiva (Ameiva atrigularis) photographed in Tobago](https://upload.wikimedia.org/wikipedia/commons/thumb/e/ef/Giant_ameiva_%28Ameiva_atrigularis%29.jpg/960px-Giant_ameiva_%28Ameiva_atrigularis%29.jpg)](https://commons.wikimedia.org/wiki/File:Giant_ameiva_(Ameiva_atrigularis).jpg)

Photo: [Charles J. Sharp](https://commons.wikimedia.org/wiki/File:Giant_ameiva_(Ameiva_atrigularis).jpg), [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/); image displayed without modifications.

## In the flock

For the toy dataset, Mato prepares the public inputs, builds shared Wikidata indexes, runs the species, and validates the resulting graph. Other dataset jobs use `mato run` with their own configured inputs and outputs.

## Start here

New to the project? Start with the [plain-language flock overview](https://github.com/oolonek/isla-gabriola-flock/blob/main/README.md). To try the complete route from download to validated graph, set `workspace` in a private copy of [the toy config](config/toy.toml), then run [the one-command toy bootstrap](TECHNICAL.md#example-download-the-zenodo-toy-dataset-and-export-a-kg). Rust and Cargo are required; Just and Python are not.

## Learn more

The [technical guide](TECHNICAL.md) covers jobs, results, resuming and configuration. The [public flock overview](https://github.com/oolonek/isla-gabriola-flock/blob/main/README.md#who-does-what) shows the other species and their current roles.

The code is available under either the [MIT license](LICENSE-MIT) or the [Apache 2.0 license](LICENSE-APACHE).

*Naming note: For new species names, we take inspiration from [colonizer](https://crates.io/crates/colonizer)'s `inspire` mode, which suggests names from the Catalogue of Life, rather than inventing each crate name ourselves.*
