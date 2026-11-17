use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use flate2::read::GzDecoder;
use md5::{Digest, Md5};
use reqwest::blocking::Client;
use tempfile::NamedTempFile;
use walkdir::WalkDir;
use zip::ZipArchive;

pub(crate) struct Archive {
    pub(crate) doi: String,
}

pub(crate) struct Download {
    pub(crate) url: String,
    pub(crate) md5: String,
    pub(crate) compression: Option<Compression>,
}

pub(crate) enum Compression {
    Gzip,
}

pub(crate) struct InputSpec {
    pub(crate) workspace_root: PathBuf,
    pub(crate) dataset_root: PathBuf,
    pub(crate) metadata: PathBuf,
    pub(crate) ms_dir: PathBuf,
    pub(crate) archive: Option<Archive>,
    pub(crate) library: PathBuf,
    pub(crate) lotus_csv: PathBuf,
    pub(crate) library_source: Option<Download>,
    pub(crate) lotus_source: Option<Download>,
}

pub(crate) fn prepare(paths: &InputSpec) -> Result<()> {
    if paths.archive.is_none() && paths.library_source.is_none() && paths.lotus_source.is_none() {
        return Ok(());
    }
    ensure!(
        paths.workspace_root.is_absolute(),
        "workspace root must be absolute: {}",
        paths.workspace_root.display()
    );
    let workspace_metadata = paths.workspace_root.symlink_metadata().with_context(|| {
        format!("workspace root must already exist: {}", paths.workspace_root.display())
    })?;
    ensure!(
        workspace_metadata.file_type().is_dir(),
        "workspace root is not a real directory: {}",
        paths.workspace_root.display()
    );
    if let Some(archive) = &paths.archive {
        let record_id = archive.doi.strip_prefix("10.5281/zenodo.")
            .filter(|id| !id.is_empty() && id.bytes().all(|byte| byte.is_ascii_digit()))
            .with_context(|| format!("archive DOI must identify a specific Zenodo release: {}", archive.doi))?;
        safe_destination(&paths.workspace_root, &paths.dataset_root, "dataset root")?;
        for (path, label) in [
            (&paths.metadata, "metadata"),
            (&paths.ms_dir, "spectra directory"),
        ] {
            safe_destination(&paths.dataset_root, path, label)?;
        }
        ensure_directory(&paths.workspace_root, &paths.dataset_root)?;
        let manifest = paths.dataset_root.join("sapayoa-manifest.jsonl");
        let receipt_present = regular_file(&manifest)?;
        let ready = receipt_present && regular_file(&paths.metadata)?
            && paths.ms_dir.symlink_metadata().is_ok_and(|meta| meta.file_type().is_dir());
        if ready {
            verified_archive(&paths.dataset_root, &archive.doi, record_id)?;
        } else {
            sapayoa::fetch(&archive.doi, &paths.dataset_root, Some(1), 1, false)
                .map_err(|error| anyhow::anyhow!("Sapayoa fetch for {} failed: {error}; keep existing data and inspect the Sapayoa manifest before retrying", archive.doi))?;
            let zip = verified_archive(&paths.dataset_root, &archive.doi, record_id)?;
            extract_verified(&paths.dataset_root, &zip)?;
        }
        ensure!(
            regular_file(&paths.metadata)?,
            "archive is missing metadata: {}",
            paths.metadata.display()
        );
        ensure!(
            paths.ms_dir.symlink_metadata().is_ok_and(|meta| meta.file_type().is_dir()),
            "archive is missing spectra directory: {}",
            paths.ms_dir.display()
        );
        ensure!(
            WalkDir::new(&paths.ms_dir).min_depth(1).into_iter().any(|entry| {
                entry.is_ok_and(|entry| entry.file_type().is_file())
            }),
            "archive has no raw spectra in {}",
            paths.ms_dir.display()
        );
    }
    let client = if paths.library_source.is_some() || paths.lotus_source.is_some() {
        Some(Client::builder()
            .user_agent(concat!("mato/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(180))
            .build()?)
    } else {
        None
    };
    if let Some(source) = &paths.library_source {
        prepare_download(client.as_ref().unwrap(), source, &paths.workspace_root, &paths.library)?;
    }
    if let Some(source) = &paths.lotus_source {
        prepare_download(client.as_ref().unwrap(), source, &paths.workspace_root, &paths.lotus_csv)?;
    }
    Ok(())
}

fn safe_destination(root: &Path, path: &Path, label: &str) -> Result<()> {
    ensure!(path.is_absolute(), "{label} path must be absolute: {}", path.display());
    let relative = path.strip_prefix(root).with_context(|| {
        format!("{label} must be inside {}: {}", root.display(), path.display())
    })?;
    ensure!(
        !relative.as_os_str().is_empty()
            && relative.components().all(|component| matches!(component, Component::Normal(_))),
        "unsafe {label} destination: {}",
        path.display()
    );
    Ok(())
}

fn prepare_download(client: &Client, source: &Download, root: &Path, target: &Path) -> Result<()> {
    safe_destination(root, target, "reference")?;
    match source.compression {
        None => download_checked(client, &source.url, &source.md5, root, target),
        Some(Compression::Gzip) => {
            let mut compressed = target.as_os_str().to_os_string();
            compressed.push(".gz");
            let compressed = PathBuf::from(compressed);
            safe_destination(root, &compressed, "compressed reference")?;
            download_checked(client, &source.url, &source.md5, root, &compressed)?;
            decompress_checked(root, &compressed, target)
        }
    }
}

// Create each component separately: create_dir_all would silently follow a preexisting symlink.
fn ensure_directory(root: &Path, directory: &Path) -> Result<()> {
    let relative = directory.strip_prefix(root).with_context(|| {
        format!("destination is outside workspace: {}", directory.display())
    })?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            bail!("unsafe destination directory: {}", directory.display());
        };
        current.push(name);
        match current.symlink_metadata() {
            Ok(metadata) => ensure!(
                metadata.file_type().is_dir(),
                "destination is not a real directory: {}",
                current.display()
            ),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&current)
                    .with_context(|| format!("creating directory {}", current.display()))?;
            }
            Err(error) => return Err(error).with_context(|| format!("checking {}", current.display())),
        }
    }
    Ok(())
}

fn regular_file(path: &Path) -> Result<bool> {
    match path.symlink_metadata() {
        Ok(metadata) => {
            ensure!(metadata.file_type().is_file(), "not a regular file: {}", path.display());
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("checking {}", path.display())),
    }
}

fn verified_archive(root: &Path, doi: &str, record_id: &str) -> Result<PathBuf> {
    let manifest = root.join("sapayoa-manifest.jsonl");
    ensure!(regular_file(&manifest)?, "Sapayoa manifest missing: {}", manifest.display());
    let mut archive = None;
    for line in BufReader::new(File::open(&manifest)?).lines() {
        let receipt: sapayoa::Receipt = serde_json::from_str(&line?)
            .with_context(|| format!("invalid Sapayoa manifest {}", manifest.display()))?;
        ensure!(
            receipt.doi == doi
                && receipt.accession.strip_prefix("zenodo:") == Some(record_id)
                && !receipt.version.is_empty()
                && receipt.path.extension().is_some_and(|ext| ext == "zip")
                && !receipt.path.as_os_str().is_empty()
                && receipt.path.components().all(|component| matches!(component, Component::Normal(_))),
            "Sapayoa manifest does not track a ZIP from the requested release: {}",
            manifest.display()
        );
        ensure!(archive.is_none(), "expected exactly one ZIP receipt in {}", manifest.display());
        archive = Some(root.join(receipt.path));
    }
    let archive = archive.with_context(|| format!("expected exactly one ZIP receipt in {}", manifest.display()))?;
    ensure!(regular_file(&archive)?, "archive missing: {}", archive.display());
    Ok(archive)
}

fn checked_entry_name(name: &[u8]) -> Result<PathBuf> {
    let name = std::str::from_utf8(name).context("ZIP entry name is not UTF-8")?;
    ensure!(
        !name.contains(['\\', ':', '\r', '\n', '\0'])
            && name.split('/').filter(|part| !part.is_empty()).all(|part| part != "." && part != "..")
            && !name.starts_with('/')
            && !name.contains("//"),
        "unsafe ZIP entry name: {name:?}"
    );
    let relative = Path::new(name);
    ensure!(
        relative.components().all(|component| matches!(component, Component::Normal(_)))
            && !relative.as_os_str().is_empty(),
        "unsafe ZIP entry path: {name:?}"
    );
    Ok(relative.to_path_buf())
}

fn extract_verified(root: &Path, archive_path: &Path) -> Result<()> {
    let mut archive = ZipArchive::new(File::open(archive_path)?)
        .with_context(|| format!("opening ZIP {}", archive_path.display()))?;
    let mut entries = Vec::with_capacity(archive.len());
    let mut explicit = HashMap::new();
    let manifest = Path::new("sapayoa-manifest.jsonl");
    let archive_name = archive_path.strip_prefix(root)?;
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        let relative = checked_entry_name(entry.name_raw())?;
        ensure!(
            relative != manifest && relative != archive_name
                && (entry.is_dir() || (!manifest.starts_with(&relative) && !archive_name.starts_with(&relative))),
            "ZIP entry conflicts with Sapayoa receipt or archive: {}",
            relative.display()
        );
        if let Some(mode) = entry.unix_mode() {
            let kind = mode & 0o170000;
            ensure!(
                kind == 0 || (entry.is_dir() && kind == 0o040000)
                    || (!entry.is_dir() && kind == 0o100000),
                "ZIP entry has unsafe file type: {}",
                relative.display()
            );
        }
        ensure!(
            explicit.insert(relative.clone(), entry.is_dir()).is_none(),
            "duplicate ZIP entry: {}",
            relative.display()
        );
        entries.push((relative, entry.is_dir()));
    }
    ensure!(!entries.is_empty(), "ZIP is empty: {}", archive_path.display());
    for (relative, _) in &entries {
        for ancestor in relative.ancestors().skip(1) {
            ensure!(
                explicit.get(ancestor).is_none_or(|is_dir| *is_dir),
                "ZIP file is also a parent directory: {}",
                ancestor.display()
            );
        }
    }
    for (index, (relative, is_dir)) in entries.into_iter().enumerate() {
        let target = root.join(&relative);
        if is_dir {
            ensure_directory(root, &target)?;
            continue;
        }
        ensure_directory(root, target.parent().context("ZIP entry has no parent")?)?;
        let mut entry = archive.by_index(index)?;
        if regular_file(&target)? {
            let mut existing = BufReader::new(File::open(&target)?);
            ensure!(
                streams_equal(&mut entry, &mut existing)?,
                "extracted file differs from verified ZIP; refusing to overwrite: {}",
                target.display()
            );
        } else {
            let mut temporary = NamedTempFile::new_in(target.parent().unwrap())?;
            io::copy(&mut entry, &mut temporary)
                .with_context(|| format!("extracting {}", relative.display()))?;
            temporary.as_file_mut().sync_all()?;
            temporary.persist_noclobber(&target).with_context(|| {
                format!("publishing extracted file without replacing {}", target.display())
            })?;
        }
    }
    Ok(())
}

// Reading to EOF also validates the ZIP CRC or gzip trailer for matching inputs.
fn streams_equal(expected: &mut impl Read, existing: &mut impl Read) -> Result<bool> {
    let mut buffer = [0_u8; 65536];
    let mut current = [0_u8; 65536];
    loop {
        let count = expected.read(&mut buffer)?;
        if count == 0 {
            return Ok(existing.read(&mut buffer[..1])? == 0);
        }
        match existing.read_exact(&mut current[..count]) {
            Ok(()) if current[..count] == buffer[..count] => {}
            Ok(()) => return Ok(false),
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(false),
            Err(error) => return Err(error.into()),
        }
    }
}

fn download_checked(client: &Client, url: &str, checksum: &str, root: &Path, target: &Path) -> Result<()> {
    ensure_directory(root, target.parent().context("reference has no parent")?)?;
    if regular_file(target)? {
        ensure!(
            md5_file(target)? == checksum,
            "existing reference has wrong MD5; refusing to replace {} (expected {checksum})",
            target.display()
        );
        return Ok(());
    }
    let mut response = client.get(url).send()?.error_for_status()
        .with_context(|| format!("downloading {url}"))?;
    let mut temporary = NamedTempFile::new_in(target.parent().unwrap())?;
    let mut hasher = Md5::new();
    let mut buffer = [0_u8; 65536];
    loop {
        let count = response.read(&mut buffer)?;
        if count == 0 { break; }
        hasher.update(&buffer[..count]);
        temporary.write_all(&buffer[..count])?;
    }
    ensure!(
        format!("{:x}", hasher.finalize()) == checksum,
        "downloaded reference has wrong MD5; refusing to publish {}",
        target.display()
    );
    temporary.as_file_mut().sync_all()?;
    temporary.persist_noclobber(target).with_context(|| {
        format!("publishing reference without replacing {}", target.display())
    })?;
    Ok(())
}

fn md5_file(path: &Path) -> Result<String> {
    let mut file = BufReader::new(File::open(path)?);
    let mut hasher = Md5::new();
    let mut buffer = [0_u8; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 { break; }
        hasher.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn decompress_checked(root: &Path, compressed: &Path, target: &Path) -> Result<()> {
    ensure_directory(root, target.parent().context("reference destination has no parent")?)?;
    let mut decoded = GzDecoder::new(BufReader::new(File::open(compressed)?));
    if regular_file(target)? {
        let mut existing = BufReader::new(File::open(target)?);
        ensure!(
            streams_equal(&mut decoded, &mut existing)?,
            "existing reference differs from verified gzip; refusing to replace {}",
            target.display()
        );
    } else {
        let mut temporary = NamedTempFile::new_in(target.parent().unwrap())?;
        io::copy(&mut decoded, &mut temporary)
            .with_context(|| format!("decompressing reference from {}", compressed.display()))?;
        temporary.as_file_mut().sync_all()?;
        temporary.persist_noclobber(target).with_context(|| {
            format!("publishing reference without replacing {}", target.display())
        })?;
    }
    Ok(())
}
