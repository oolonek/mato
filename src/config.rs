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
    KgExport,
    KgValidate,
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
            Self::KgExport => "kg_export",
            Self::KgValidate => "kg_validate",
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
            Self::KgExport => "exports the database as a linked N-Quads graph",
            Self::KgValidate => "validates the exported graph against SHACL shapes",
        }
    }

    pub(crate) fn species(self) -> &'static str {
        match self {
            Self::Validate => "machaca",
            Self::Ingest | Self::Register | Self::KgExport | Self::KgValidate => "peregrino",
            Self::Enrich => "cunaguaro",
            Self::Picoto => "picoto",
            Self::Annotate => "hocofaisan",
            Self::ChemicalEnrich => "cunaguaro",
        }
    }

    pub(crate) fn emoji(self) -> &'static str {
        match self {
            Self::Validate => "🦗",
            Self::Ingest | Self::Register | Self::Picoto | Self::KgExport | Self::KgValidate => "🦈",
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
    #[serde(default)]
    archive: Option<Archive>,
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
    #[serde(default)]
    create_missing: bool,
}


pub(crate) struct SharedIndexes {
    pub(crate) taxon_index: PathBuf,
    pub(crate) chemical_index: PathBuf,
    pub(crate) taxon_endpoint: Option<String>,
    pub(crate) chemical_endpoint: Option<String>,
}

impl SharedIndexes {
    pub(crate) fn load(path: &Path) -> Result<Self> {
        let (_, dir, file) = load_file(path)?;
        let workspace = comparison_path(&relative_to(&dir, &file.workspace))?;
        Ok(Self {
            taxon_index: relative_to(&workspace, &file.indexes.taxon_index),
            chemical_index: relative_to(&workspace, &file.indexes.chemical_index),
            taxon_endpoint: file.indexes.taxon_endpoint,
            chemical_endpoint: file.indexes.chemical_endpoint,
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
    #[serde(default)]
    library_source: Option<Source>,
    #[serde(default)]
    lotus_source: Option<Source>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KnowledgeGraph {
    output_dir: PathBuf,
    #[serde(default)]
    allow_partial: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Archive {
    doi: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    url: String,
    md5: String,
    compression: Option<Compression>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum Compression {
    Gzip,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct QleverFile {
    index_root: Option<PathBuf>,
    ui_data: Option<PathBuf>,
    ui_image: Option<String>,
    engine_image: Option<String>,
    ui_port: Option<u16>,
}

pub(crate) struct QleverSettings {
    pub(crate) index_root: PathBuf,
    pub(crate) ui_data: PathBuf,
    pub(crate) ui_image: String,
    pub(crate) engine_image: String,
    pub(crate) ui_port: u16,
}

impl QleverSettings {
    pub(crate) fn load(path: &Path) -> Result<Self> {
        let (_, dir, file) = load_file(path)?;
        let workspace = existing_dir(&relative_to(&dir, &file.workspace), "workspace")?;
        let source = existing_dir(&relative_to(&dir, &file.flock_root), "flock root")?;
        let dataset = comparison_path(&relative_to(&workspace, &file.dataset.root))?;
        let configured = file.qlever.unwrap_or_default();
        let index_root = comparison_path(&relative_to(
            &workspace,
            configured.index_root.as_deref().unwrap_or(Path::new("mato/qlever-indexes")),
        ))?;
        let ui_data = comparison_path(&relative_to(
            &workspace,
            configured.ui_data.as_deref().unwrap_or(Path::new("mato/qlever-ui")),
        ))?;
        for path in [&index_root, &ui_data] {
            ensure!(
                path.starts_with(&workspace) && !path.starts_with(&dataset) && !path.starts_with(&source),
                "QLever output must be inside workspace, outside dataset and source trees: {}",
                path.display()
            );
        }
        ensure!(
            !index_root.starts_with(&ui_data) && !ui_data.starts_with(&index_root),
            "QLever index and shared UI directories must be separate"
        );
        let ui_port = configured.ui_port.unwrap_or(7044);
        ensure!(ui_port >= 1024, "QLever UI port must be at least 1024");
        let ui_image = configured.ui_image.unwrap_or_else(|| "qleverui:latest".to_owned());
        let engine_image = configured.engine_image.unwrap_or_else(|| {
            "docker.io/adfreiburg/qlever@sha256:27d51763ca24bfe49a4f9e4108babae376f2dffef1313086b95bd262f06e6492".to_owned()
        });
        ensure!(!ui_image.trim().is_empty() && !engine_image.trim().is_empty(), "QLever image references cannot be empty");
        Ok(Self { index_root, ui_data, ui_image, engine_image, ui_port })
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    workspace: PathBuf,
    flock_root: PathBuf,
    pipeline: Pipeline,
    dataset: Dataset,
    database: Database,
    picoto: Picoto,
    indexes: Indexes,
    annotation: Annotation,
    knowledge_graph: KnowledgeGraph,
    #[serde(default)]
    peregrino: Option<Peregrino>,
    #[serde(default)]
    qlever: Option<QleverFile>,
    #[serde(default)]
    selection: Option<crate::selection::Selection>,
}

pub(crate) struct Config {
    pub(crate) path: PathBuf,
    pub(crate) flock_root: PathBuf,
    pub(crate) stages: Vec<Stage>,
    pub(crate) top_n: Option<usize>,
    pub(crate) dataset_root: PathBuf,
    pub(crate) doi: String,
    pub(crate) metadata: PathBuf,
    /// Original source metadata, when `metadata` is a selected immutable snapshot.
    pub(crate) source_metadata: Option<PathBuf>,
    pub(crate) selection: Option<crate::selection::Selection>,
    pub(crate) ms_dir: PathBuf,
    pub(crate) db: PathBuf,
    pub(crate) output_dir: PathBuf,
    pub(crate) picoto_jobs: usize,
    pub(crate) taxon_index: PathBuf,
    pub(crate) peregrino_jobs: usize,
    pub(crate) library: PathBuf,
    pub(crate) lotus_csv: PathBuf,
    pub(crate) settings: PathBuf,
    pub(crate) chemical_index: PathBuf,
    pub(crate) graph_dir: PathBuf,
    pub(crate) allow_partial: bool,
    pub(crate) create_missing_indexes: bool,
}


fn load_file(path: &Path) -> Result<(PathBuf, PathBuf, FileConfig)> {
    let path = existing_file(path, "job config")?;
    let dir = path.parent().context("job config has no parent")?.to_path_buf();
    let file = toml::from_str(&std::fs::read_to_string(&path)?)
        .with_context(|| format!("parsing {}", path.display()))?;
    Ok((path, dir, file))
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
                "{key} must be unique and in this order: validate, ingest, enrich, picoto, register, annotate, chemical_enrich, kg_export, kg_validate"
            );
        }
    }
    ensure!(
        !stages.contains(&Stage::KgValidate) || stages.contains(&Stage::KgExport),
        "{key}: kg_validate requires kg_export earlier in the same job"
    );
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

/// Prepare only the inputs needed by the selected stages before checking their existence.
pub(crate) fn prepare_inputs(path: &Path, top_n: Option<usize>) -> Result<()> {
    let (_, dir, file) = load_file(path)?;
    check_stages(&file.pipeline.stages, "pipeline.stages")?;
    if let Some(top_n_stages) = &file.pipeline.top_n_stages {
        check_stages(top_n_stages, "pipeline.top_n_stages")?;
    }
    if let Some(n) = top_n {
        ensure!(n > 0 && file.database.top_n_root.is_some(), "--topN requires a positive limit and [database].top_n_root");
    }
    let stages = if top_n.is_some() {
        file.pipeline.top_n_stages.as_deref().context("--topN requires top_n_stages")?
    } else {
        &file.pipeline.stages
    };
    let archive_needed = stages.iter().any(|stage| matches!(
        stage,
        Stage::Validate | Stage::Ingest | Stage::Picoto
    ));
    let refs_needed = stages.contains(&Stage::Annotate);
    if (!archive_needed || file.dataset.archive.is_none())
        && (!refs_needed || (file.annotation.library_source.is_none() && file.annotation.lotus_source.is_none()))
    {
        return Ok(());
    }
    let workspace = existing_dir(&relative_to(&dir, &file.workspace), "workspace root")?;
    let dataset_root = comparison_path(&relative_to(&workspace, &file.dataset.root))?;
    let metadata = comparison_path(&relative_to(&dataset_root, &file.dataset.metadata))?;
    let ms_dir = comparison_path(&relative_to(&dataset_root, &file.dataset.ms_dir))?;
    let library = comparison_path(&relative_to(&workspace, &file.annotation.library))?;
    let lotus_csv = comparison_path(&relative_to(&workspace, &file.annotation.lotus_csv))?;
    let flock_root = comparison_path(&relative_to(&dir, &file.flock_root))?;
    ensure!(
        dataset_root.starts_with(&workspace) && !dataset_root.starts_with(&flock_root),
        "dataset root must be inside workspace and outside the source tree"
    );
    for input in [&metadata, &ms_dir] {
        ensure!(input.starts_with(&dataset_root), "dataset input outside dataset root: {}", input.display());
    }
    for reference in [&library, &lotus_csv] {
        ensure!(
            reference.starts_with(&workspace) && !reference.starts_with(&dataset_root)
                && !reference.starts_with(&flock_root),
            "reference must be inside workspace, outside dataset and source trees: {}",
            reference.display()
        );
    }
    let archive = archive_needed.then_some(file.dataset.archive).flatten().map(|archive| {
        crate::inputs::Archive { doi: archive.doi }
    });
    let download = |source: Source| crate::inputs::Download {
        url: source.url,
        md5: source.md5,
        compression: source.compression.map(|compression| match compression {
            Compression::Gzip => crate::inputs::Compression::Gzip,
        }),
    };
    crate::inputs::prepare(&crate::inputs::InputSpec {
        workspace_root: workspace,
        dataset_root,
        metadata,
        ms_dir,
        archive,
        library,
        lotus_csv,
        library_source: refs_needed.then_some(file.annotation.library_source).flatten().map(download),
        lotus_source: refs_needed.then_some(file.annotation.lotus_source).flatten().map(download),
    })
}

impl Config {
    pub(crate) fn load(path: &Path, top_n: Option<usize>) -> Result<Self> {
        if let Some(n) = top_n {
            ensure!(n > 0, "--topN must be greater than zero");
        }
        let (path, dir, file) = load_file(path)?;
        let workspace = comparison_path(&relative_to(&dir, &file.workspace))?;
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

        let flock_root = existing_dir(&relative_to(&dir, &file.flock_root), "flock root")?;
        let dataset_root = existing_dir(&relative_to(&workspace, &file.dataset.root), "dataset root")?;
        let source_metadata = inside_dataset(&dataset_root, &file.dataset.metadata, false)?;
        let selected = if let (Some(selection), Some(limit)) = (file.selection.as_ref(), top_n) {
            let snapshot = crate::selection::path(&dataset_root, &source_metadata, selection, limit)?;
            let resolved = comparison_path(&snapshot)?;
            ensure!(resolved.starts_with(&dataset_root) && resolved != source_metadata,
                "selected metadata must stay within the dataset, separate from its source: {}", snapshot.display());
            Some(resolved)
        } else {
            None
        };
        let metadata = selected.as_deref().unwrap_or(&source_metadata).to_path_buf();
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
            let root = relative_to(&workspace, root);
            let base = root.join(n.to_string());
            let db = base.join("flock.sqlite");
            let output_dir = base.join("picoto");
            let production_db = comparison_path(&relative_to(&workspace, &file.database.path))?;
            let production_output = comparison_path(&relative_to(&workspace, &file.picoto.output_dir))?;
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
                relative_to(&workspace, &file.database.path),
                relative_to(&workspace, &file.picoto.output_dir),
            )
        };
        let graph_root = comparison_path(&relative_to(&workspace, &file.knowledge_graph.output_dir))?;
        let graph_dir = if let Some(n) = top_n {
            comparison_path(&graph_root.join(n.to_string()))?
        } else {
            graph_root.clone()
        };
        let db_safety = comparison_path(&db)?;
        let picoto_safety = comparison_path(&output_dir)?;
        ensure!(
            graph_dir.starts_with(&workspace)
                && !graph_dir.starts_with(&flock_root)
                && !graph_dir.starts_with(&dataset_root)
                && !graph_dir.starts_with(&picoto_safety)
                && !picoto_safety.starts_with(&graph_dir)
                && !db_safety.starts_with(&graph_dir)
                && !graph_dir.starts_with(&db_safety),
            "knowledge graph output must be separate from source, dataset, database and Picoto directories"
        );
        let annotation = stages.contains(&Stage::Annotate);
        let library = relative_to(&workspace, &file.annotation.library);
        let lotus_csv = relative_to(&workspace, &file.annotation.lotus_csv);
        let settings = relative_to(&dir, &file.annotation.settings);
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
            path: path.clone(),
            flock_root,
            stages,
            top_n,
            dataset_root,
            doi: file.dataset.doi,
            metadata,
            source_metadata: selected.map(|_| source_metadata),
            selection: top_n.and(file.selection),
            ms_dir,
            db,
            output_dir,
            picoto_jobs: file.picoto.jobs,
            taxon_index: relative_to(&workspace, &file.indexes.taxon_index),
            peregrino_jobs,
            library,
            lotus_csv,
            settings,
            chemical_index: relative_to(&workspace, &file.indexes.chemical_index),
            graph_dir,
            allow_partial: file.knowledge_graph.allow_partial,
            create_missing_indexes: file.indexes.create_missing,
        })
    }
}
