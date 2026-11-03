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
            Self::ChemicalEnrich => "jaiba",
        }
    }

    pub(crate) fn emoji(self) -> &'static str {
        match self {
            Self::Validate => "🦗",
            Self::Ingest | Self::Register | Self::Picoto => "🦈",
            Self::Enrich => "🐆",
            Self::Annotate => "🦃",
            Self::ChemicalEnrich => "🦀",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Pipeline {
    stages: Vec<Stage>,
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
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Picoto {
    output_dir: PathBuf,
    jobs: usize,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cunaguaro {
    jobs: usize,
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
    pipeline: Pipeline,
    dataset: Dataset,
    database: Database,
    picoto: Picoto,
    annotation: Annotation,
    #[serde(default)]
    cunaguaro: Option<Cunaguaro>,
    #[serde(default)]
    peregrino: Option<Peregrino>,
}

pub(crate) struct Config {
    pub(crate) path: PathBuf,
    pub(crate) flock_root: PathBuf,
    pub(crate) stages: Vec<Stage>,
    pub(crate) dataset_root: PathBuf,
    pub(crate) doi: String,
    pub(crate) metadata: PathBuf,
    pub(crate) ms_dir: PathBuf,
    pub(crate) db: PathBuf,
    pub(crate) output_dir: PathBuf,
    pub(crate) picoto_jobs: usize,
    pub(crate) cunaguaro_jobs: usize,
    pub(crate) peregrino_jobs: usize,
    pub(crate) library: PathBuf,
    pub(crate) lotus_csv: PathBuf,
    pub(crate) settings: PathBuf,
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

impl Config {
    pub(crate) fn load(path: &Path) -> Result<Self> {
        let path = existing_file(path, "pipeline config")?;
        let dir = path.parent().context("pipeline config has no parent")?;
        let file: FileConfig = toml::from_str(&std::fs::read_to_string(&path)?)
            .with_context(|| format!("parsing {}", path.display()))?;
        ensure!(
            !file.pipeline.stages.is_empty(),
            "pipeline.stages must not be empty"
        );
        for pair in file.pipeline.stages.windows(2) {
            if pair[0] >= pair[1] {
                bail!(
                    "pipeline stages must be unique and in this order: validate, ingest, enrich, picoto, register, annotate, chemical_enrich"
                );
            }
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
        let cunaguaro_jobs = file.cunaguaro.map_or(1, |cunaguaro| cunaguaro.jobs);
        ensure!(
            (1..=3).contains(&cunaguaro_jobs),
            "cunaguaro.jobs must be between 1 and 3"
        );
        let peregrino_jobs = file.peregrino.map_or(1, |peregrino| peregrino.jobs);
        ensure!(peregrino_jobs > 0, "peregrino.jobs must be positive");

        let flock_root = existing_dir(&relative_to(dir, &file.flock_root), "flock root")?;
        let dataset_root = existing_dir(&relative_to(dir, &file.dataset.root), "dataset root")?;
        let metadata = inside_dataset(&dataset_root, &file.dataset.metadata, false)?;
        let ms_dir = inside_dataset(&dataset_root, &file.dataset.ms_dir, true)?;
        let db = relative_to(dir, &file.database.path);
        let output_dir = relative_to(dir, &file.picoto.output_dir);
        let annotation = file.pipeline.stages.contains(&Stage::Annotate);
        let library = relative_to(dir, &file.annotation.library);
        let lotus_csv = relative_to(dir, &file.annotation.lotus_csv);
        let settings = relative_to(dir, &file.annotation.settings);
        if annotation {
            existing_file(&library, "annotation library")?;
            existing_file(&lotus_csv, "LOTUS metadata")?;
            existing_file(&settings, "hocofaisan settings")?;
        }
        for stage in &file.pipeline.stages {
            let manifest = flock_root.join(stage.species()).join("Cargo.toml");
            existing_file(&manifest, "species manifest")?;
        }
        Ok(Self {
            path,
            flock_root,
            stages: file.pipeline.stages,
            dataset_root,
            doi: file.dataset.doi,
            metadata,
            ms_dir,
            db,
            output_dir,
            picoto_jobs: file.picoto.jobs,
            cunaguaro_jobs,
            peregrino_jobs,
            library,
            lotus_csv,
            settings,
        })
    }
}
