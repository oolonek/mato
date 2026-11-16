# mato

Mato coordinates the [Isla Gabriola flock](https://github.com/oolonek/flock-roadmap/blob/main/README.md), a group of tools that connects research samples, mass-spectrometry measurements, and possible chemical matches into a traceable record that can become a searchable knowledge graph. Mato runs the steps you choose; it does not download the data or claim that a possible match identifies a chemical.

The name Mato refers to a lizard: the giant ameiva, *[Ameiva atrigularis](https://www.wikidata.org/wiki/Q2842576)* ([Catalogue of Life: 66B76](https://www.catalogueoflife.org/data/taxon/66B76)).

[![Giant ameiva (Ameiva atrigularis) photographed in Tobago](https://upload.wikimedia.org/wikipedia/commons/thumb/e/ef/Giant_ameiva_%28Ameiva_atrigularis%29.jpg/960px-Giant_ameiva_%28Ameiva_atrigularis%29.jpg)](https://commons.wikimedia.org/wiki/File:Giant_ameiva_(Ameiva_atrigularis).jpg)

Photo: [Charles J. Sharp](https://commons.wikimedia.org/wiki/File:Giant_ameiva_(Ameiva_atrigularis).jpg), [CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/); image displayed without modifications.

## In the flock

After the data is downloaded and extracted, Mato asks the other species to check the files, record samples, find signals and compare spectra with references. Peregrino can then export a knowledge graph from the resulting record.

## Start here

New to the project? Start with the [plain-language flock overview](https://github.com/oolonek/flock-roadmap/blob/main/README.md) to see why the species work together. To try the complete route from download to graph, follow the [small Zenodo toy walkthrough](TECHNICAL.md#example-download-the-zenodo-toy-dataset-and-export-a-kg); it lists the data, reference files and safety checks you need first.

## Learn more

The [technical guide](TECHNICAL.md) covers jobs, results, resuming and configuration. The [flock roadmap](https://github.com/oolonek/flock-roadmap/blob/main/ROADMAP.md#flock-at-a-glance) shows the other species and their handoffs.

*Naming note: For new species names, we take inspiration from [colonizer](https://crates.io/crates/colonizer)'s `inspire` mode, which suggests names from the Catalogue of Life, rather than inventing each crate name ourselves.*
