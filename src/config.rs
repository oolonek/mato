use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Stage {
    Validate,
    Ingest,
    Enrich,
    Picoto,
    Register,
    Annotate,
    ChemicalEnrich,
}

impl Stage {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Validate => "validate",
            Self::Ingest => "ingest",
            Self::Enrich => "enrich",
            Self::Picoto => "picoto",
            Self::Register => "register",
            Self::Annotate => "annotate",
            Self::ChemicalEnrich => "chemical_enrich",
        }
    }

    pub(crate) fn description(self) -> &'static str {
        match self {
            Self::Validate => "validates metadata against raw files",
            Self::Ingest => "imports metadata, spectra and taxa",
            Self::Enrich => "matches Wikidata names and stores IDs/errors",
            Self::Picoto => "extracts features and MGF files",
            Self::Register => "registers Picoto results",
            Self::Annotate => "annotates positive-mode spectra with LOTUS taxonomy",
            Self::ChemicalEnrich => "resolves current library candidates by exact full InChIKey",
        }
    }

    pub(crate) fn species(self) -> &'static str {
        match self {
            Self::Validate => "machaca",
            Self::Ingest | Self::Register => "peregrino",
            Self::Enrich => "cunaguaro",
            Self::Picoto => "picoto",
            Self::Annotate => "hocofaisan",
            Self::ChemicalEnrich => "cunaguaro",
        }
    }

    pub(crate) fn emoji(self) -> &'static str {
        match self {
            Self::Validate => "🦗",
            Self::Ingest | Self::Register | Self::Picoto => "🦈",
            Self::Enrich => "🐆",
            Self::Annotate => "🦃",
            Self::ChemicalEnrich => "🐆",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pipeline {
    stages: Vec<Stage>,
    #[serde(default)]
    top_n_stages: Option<Vec<Stage>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Dataset {
    root: PathBuf,
    doi: String,
    metadata: PathBuf,
    ms_dir: PathBuf,
    taxon_column: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Database {
    path: PathBuf,
    #[serde(default)]
    top_n_root: Option<PathBuf>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Picoto {
    output_dir: PathBuf,
    jobs: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Indexes {
    taxon_index: PathBuf,
    chemical_index: PathBuf,
    #[serde(default)]
    taxon_endpoint: Option<String>,
    #[serde(default)]
    chemical_endpoint: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct IndexesFile {
    indexes: Indexes,
}

pub(crate) struct SharedIndexes {
    pub(crate) path: PathBuf,
    pub(crate) taxon_index: PathBuf,
    pub(crate) chemical_index: PathBuf,
    pub(crate) taxon_endpoint: Option<String>,
    pub(crate) chemical_endpoint: Option<String>,
}

impl SharedIndexes {
    pub(crate) fn load(path: &Path) -> Result<Self> {
        let path = existing_file(path, "Wikidata index config")?;
        let dir = path.parent().context("Wikidata index config has no parent")?;
        let file: IndexesFile = toml::from_str(&std::fs::read_to_string(&path)?)
            .with_context(|| format!("parsing {}", path.display()))?;
        Ok(Self {
            taxon_index: relative_to(dir, &file.indexes.taxon_index),
            chemical_index: relative_to(dir, &file.indexes.chemical_index),
            taxon_endpoint: file.indexes.taxon_endpoint,
            chemical_endpoint: file.indexes.chemical_endpoint,
            path,
        })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Peregrino {
    jobs: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Annotation {
    library: PathBuf,
    lotus_csv: PathBuf,
    settings: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    flock_root: PathBuf,
    #[serde(default)]
    wikidata_config: Option<PathBuf>,
    pipeline: Pipeline,
    dataset: Dataset,
    database: Database,
    picoto: Picoto,
    annotation: Annotation,
    #[serde(default)]
    peregrino: Option<Peregrino>,
}

pub(crate) struct Config {
    pub(crate) path: PathBuf,
    pub(crate) flock_root: PathBuf,
    pub(crate) stages: Vec<Stage>,
    pub(crate) top_n: Option<usize>,
    pub(crate) dataset_root: PathBuf,
    pub(crate) doi: String,
    pub(crate) metadata: PathBuf,
    pub(crate) ms_dir: PathBuf,
    pub(crate) db: PathBuf,
    pub(crate) output_dir: PathBuf,
    pub(crate) picoto_jobs: usize,
    pub(crate) indexes_config: PathBuf,
    pub(crate) taxon_index: PathBuf,
    pub(crate) peregrino_jobs: usize,
    pub(crate) library: PathBuf,
    pub(crate) lotus_csv: PathBuf,
    pub(crate) settings: PathBuf,
    pub(crate) chemical_index: PathBuf,
}

fn relative_to(base: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base.join(path)
    }
}

fn existing_file(path: &Path, what: &str) -> Result<PathBuf> {
    let canonical = path
        .canonicalize()
        .with_context(|| format!("{what}: {}", path.display()))?;
    ensure!(
        canonical.is_file(),
        "{what} must be a file: {}",
        canonical.display()
    );
    Ok(canonical)
}

fn existing_dir(path: &Path, what: &str) -> Result<PathBuf> {
    let canonical = path
        .canonicalize()
        .with_context(|| format!("{what}: {}", path.display()))?;
    ensure!(
        canonical.is_dir(),
        "{what} must be a directory: {}",
        canonical.display()
    );
    Ok(canonical)
}

fn inside_dataset(root: &Path, path: &Path, is_dir: bool) -> Result<PathBuf> {
    let candidate = relative_to(root, path);
    let resolved = if is_dir {
        existing_dir(&candidate, "spectrum directory")?
    } else {
        existing_file(&candidate, "sample metadata")?
    };
    ensure!(
        resolved.starts_with(root),
        "{} is outside dataset root {}",
        resolved.display(),
        root.display()
    );
    Ok(resolved)
}

fn check_stages(stages: &[Stage], key: &str) -> Result<()> {
    ensure!(!stages.is_empty(), "{key} must not be empty");
    for pair in stages.windows(2) {
        if pair[0] >= pair[1] {
            bail!(
                "{key} must be unique and in this order: validate, ingest, enrich, picoto, register, annotate, chemical_enrich"
            );
        }
    }
    Ok(())
}

// Resolve existing symlinks as each path component is traversed without
// creating the output directories; this also handles `..` after a symlink.
fn comparison_path(path: &Path) -> Result<PathBuf> {
    let mut resolved = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                resolved.pop();
            }
            std::path::Component::CurDir => {}
            other => {
                resolved.push(other.as_os_str());
                if resolved.exists() {
                    resolved = resolved
                        .canonicalize()
                        .with_context(|| format!("resolving output path {}", path.display()))?;
                }
            }
        }
    }
    Ok(resolved)
}

impl Config {
    pub(crate) fn load(path: &Path, top_n: Option<usize>) -> Result<Self> {
        if let Some(n) = top_n {
            ensure!(n > 0, "--topN must be greater than zero");
        }
        let path = existing_file(path, "pipeline config")?;
        let dir = path.parent().context("pipeline config has no parent")?;
        let file: FileConfig = toml::from_str(&std::fs::read_to_string(&path)?)
            .with_context(|| format!("parsing {}", path.display()))?;
        check_stages(&file.pipeline.stages, "pipeline.stages")?;
        if let Some(stages) = &file.pipeline.top_n_stages {
            check_stages(stages, "pipeline.top_n_stages")?;
        }
        if top_n.is_some() {
            ensure!(
                file.pipeline.top_n_stages.is_some() && file.database.top_n_root.is_some(),
                "--topN requires both [pipeline].top_n_stages and [database].top_n_root in the job config"
            );
        }
        ensure!(
            !file.dataset.doi.trim().is_empty(),
            "dataset.doi must not be empty"
        );
        ensure!(
            !file.dataset.taxon_column.trim().is_empty(),
            "dataset.taxon_column must not be empty"
        );
        ensure!(file.picoto.jobs > 0, "picoto.jobs must be positive");
        let peregrino_jobs = file.peregrino.map_or(1, |peregrino| peregrino.jobs);
        ensure!(peregrino_jobs > 0, "peregrino.jobs must be positive");

        let flock_root = existing_dir(&relative_to(dir, &file.flock_root), "flock root")?;
        let indexes_path = file.wikidata_config.as_deref().map_or_else(
            || flock_root.join("mato/config/wikidata.toml"),
            |path| relative_to(dir, path),
        );
        let indexes = SharedIndexes::load(&indexes_path)?;
        let dataset_root = existing_dir(&relative_to(dir, &file.dataset.root), "dataset root")?;
        let metadata = inside_dataset(&dataset_root, &file.dataset.metadata, false)?;
        let ms_dir = inside_dataset(&dataset_root, &file.dataset.ms_dir, true)?;
        let (stages, db, output_dir) = if let Some(n) = top_n {
            let stages = file
                .pipeline
                .top_n_stages
                .context("--topN requires [pipeline].top_n_stages in the job config")?;
            let root = file
                .database
                .top_n_root
                .as_deref()
                .context("--topN requires [database].top_n_root in the job config")?;
            let root = relative_to(dir, root);
            let base = root.join(n.to_string());
            let db = base.join("flock.sqlite");
            let output_dir = base.join("picoto");
            let production_db = comparison_path(&relative_to(dir, &file.database.path))?;
            let production_output = comparison_path(&relative_to(dir, &file.picoto.output_dir))?;
            let limited_root = comparison_path(&root)?;
            let limited_base = comparison_path(&base)?;
            let limited_db = comparison_path(&db)?;
            let limited_output = comparison_path(&output_dir)?;
            for path in [&limited_root, &limited_base, &limited_db, &limited_output] {
                ensure!(
                    !path.starts_with(&dataset_root) && !path.starts_with(&flock_root),
                    "--topN root and outputs must be outside dataset and flock source trees"
                );
            }
            ensure!(
                limited_db != production_db
                    && !limited_db.starts_with(&production_db)
                    && !limited_output.starts_with(&production_db)
                    && !limited_root.starts_with(&production_output)
                    && !limited_base.starts_with(&production_output)
                    && !limited_db.starts_with(&production_output)
                    && !limited_output.starts_with(&production_output)
                    && !production_output.starts_with(&limited_output)
                    && !production_db.starts_with(&limited_output),
                "--topN database and Picoto paths must be separate from production paths"
            );
            (stages, db, output_dir)
        } else {
            (
                file.pipeline.stages,
                relative_to(dir, &file.database.path),
                relative_to(dir, &file.picoto.output_dir),
            )
        };
        let annotation = stages.contains(&Stage::Annotate);
        let library = relative_to(dir, &file.annotation.library);
        let lotus_csv = relative_to(dir, &file.annotation.lotus_csv);
        let settings = relative_to(dir, &file.annotation.settings);
        if annotation {
            existing_file(&library, "annotation library")?;
            existing_file(&lotus_csv, "LOTUS metadata")?;
            existing_file(&settings, "hocofaisan settings")?;
        }
        for stage in &stages {
            let manifest = flock_root.join(stage.species()).join("Cargo.toml");
            existing_file(&manifest, "species manifest")?;
        }
        Ok(Self {
            path,
            flock_root,
            stages,
            top_n,
            dataset_root,
            doi: file.dataset.doi,
            metadata,
            ms_dir,
            db,
            output_dir,
            picoto_jobs: file.picoto.jobs,
            indexes_config: indexes.path,
            taxon_index: indexes.taxon_index,
            peregrino_jobs,
            library,
            lotus_csv,
            settings,
            chemical_index: indexes.chemical_index,
        })
    }
}
