use std::collections::{HashMap, HashSet};
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

use crate::config::BootstrapPaths;

const CONCEPT_DOI: &str = "10.5281/zenodo.10018264";
// Fetch the immutable release, not the concept DOI (which can resolve to a newer release).
const RELEASE_DOI: &str = "10.5281/zenodo.10018590";
const DATASET_DIR: &str = "sapayoa/zenodo-10018590";
const LIBRARY_URL: &str =
    "https://zenodo.org/records/14887271/files/isdb_lotus_pos_energySum.mgf?download=1";
const LIBRARY_MD5: &str = "c025e3365ca913b46ec4f6637d381911";
const LOTUS_URL: &str =
    "https://zenodo.org/records/7534071/files/230106_frozen_metadata.csv.gz?download=1";
const LOTUS_MD5: &str = "324579ae4a4785bf1b50b532b0a00eec";

pub(crate) fn prepare(paths: &BootstrapPaths) -> Result<()> {
    ensure!(
        paths.doi == CONCEPT_DOI,
        "toy bootstrap requires dataset DOI {CONCEPT_DOI}, got {}",
        paths.doi
    );
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
    ensure!(
        paths.dataset_root == paths.workspace_root.join(DATASET_DIR),
        "toy dataset root must be {DATASET_DIR} inside the configured workspace: {}",
        paths.dataset_root.display()
    );
    for (path, label) in [
        (&paths.library, "spectral library"),
        (&paths.lotus_csv, "LOTUS CSV"),
    ] {
        ensure!(path.is_absolute(), "{label} path must be absolute: {}", path.display());
        let relative = path.strip_prefix(&paths.workspace_root).with_context(|| {
            format!("{label} must be inside workspace: {}", path.display())
        })?;
        ensure!(
            relative.components().all(|component| matches!(component, Component::Normal(_)))
                && !relative.as_os_str().is_empty()
                && !path.starts_with(&paths.dataset_root),
            "unsafe {label} destination: {}",
            path.display()
        );
    }
    ensure!(
        paths.library != paths.lotus_csv && paths.lotus_csv.extension().is_some_and(|ext| ext == "csv"),
        "reference paths must be distinct, and the LOTUS destination must end in .csv"
    );
    let mut compressed = paths.lotus_csv.as_os_str().to_os_string();
    compressed.push(".gz");
    let compressed = PathBuf::from(compressed);
    ensure!(
        compressed != paths.library,
        "spectral library and compressed LOTUS file must have different paths"
    );
    ensure_directory(&paths.workspace_root, &paths.dataset_root)?;
    // Sapayoa appends receipts; never let an existing manifest redirect that write.
    regular_file(&paths.dataset_root.join("sapayoa-manifest.jsonl"))?;

    sapayoa::fetch(RELEASE_DOI, &paths.dataset_root, None, 1, false)
        .map_err(|error| anyhow::anyhow!("Sapayoa fetch for pinned {RELEASE_DOI} failed: {error}; keep existing data and inspect the Sapayoa manifest before retrying"))?;
    let archive = single_archive(&paths.dataset_root)?;
    verify_manifest(&paths.dataset_root, &archive)?;
    extract_verified(&paths.dataset_root, &archive)?;
    ensure!(
        regular_file(&paths.dataset_root.join("metadata/metadata.tsv"))?,
        "toy archive is missing metadata/metadata.tsv in {}",
        paths.dataset_root.display()
    );
    let raw = paths.dataset_root.join("msdata/raw");
    ensure!(
        raw.symlink_metadata().is_ok_and(|meta| meta.file_type().is_dir()),
        "toy archive is missing msdata/raw in {}",
        paths.dataset_root.display()
    );
    ensure!(
        WalkDir::new(&raw).min_depth(1).into_iter().any(|entry| {
            entry.is_ok_and(|entry| entry.file_type().is_file())
        }),
        "toy archive has no raw spectra in {}",
        raw.display()
    );

    let client = Client::builder()
        .user_agent(concat!("mato/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(180))
        .build()?;
    download_checked(&client, LIBRARY_URL, LIBRARY_MD5, &paths.workspace_root, &paths.library)?;
    download_checked(&client, LOTUS_URL, LOTUS_MD5, &paths.workspace_root, &compressed)?;
    decompress_checked(&paths.workspace_root, &compressed, &paths.lotus_csv)?;
    Ok(())
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

fn single_archive(root: &Path) -> Result<PathBuf> {
    let mut archive = None;
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "zip") {
            ensure!(regular_file(&path)?, "archive missing: {}", path.display());
            ensure!(archive.is_none(), "multiple Zenodo ZIP archives in {}", root.display());
            archive = Some(path);
        }
    }
    archive.with_context(|| format!("expected exactly one Zenodo ZIP archive in {}", root.display()))
}

fn verify_manifest(root: &Path, archive: &Path) -> Result<()> {
    let manifest = root.join("sapayoa-manifest.jsonl");
    ensure!(regular_file(&manifest)?, "Sapayoa manifest missing: {}", manifest.display());
    let archive_name = archive.strip_prefix(root)?;
    let mut found = false;
    for line in BufReader::new(File::open(&manifest)?).lines() {
        let receipt: sapayoa::Receipt = serde_json::from_str(&line?)
            .with_context(|| format!("invalid Sapayoa manifest {}", manifest.display()))?;
        ensure!(
            receipt.doi == RELEASE_DOI && receipt.accession == "zenodo:10018590"
                && !receipt.version.is_empty(),
            "Sapayoa manifest is not the pinned Zenodo record 10018590; preserve the existing dataset and choose a separate workspace for this toy release"
        );
        if receipt.path == archive_name {
            ensure!(!found, "duplicate archive receipts in {}", manifest.display());
            found = true;
        }
    }
    ensure!(found, "Zenodo ZIP is not tracked by Sapayoa: {}", archive.display());
    Ok(())
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
            && relative.components().next().is_some_and(|component| {
                component.as_os_str() == "metadata" || component.as_os_str() == "msdata"
            }),
        "unexpected ZIP entry path: {name:?}"
    );
    Ok(relative.to_path_buf())
}

fn extract_verified(root: &Path, archive_path: &Path) -> Result<()> {
    let mut archive = ZipArchive::new(File::open(archive_path)?)
        .with_context(|| format!("opening ZIP {}", archive_path.display()))?;
    let mut entries = Vec::with_capacity(archive.len());
    let mut explicit = HashMap::new();
    let mut known = HashSet::new();
    known.insert(PathBuf::from("sapayoa-manifest.jsonl"));
    known.insert(archive_path.strip_prefix(root)?.to_path_buf());
    for index in 0..archive.len() {
        let entry = archive.by_index(index)?;
        let relative = checked_entry_name(entry.name_raw())?;
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
        known.insert(relative.clone());
        for ancestor in relative.ancestors().skip(1) {
            if ancestor.as_os_str().is_empty() { break; }
            known.insert(ancestor.to_path_buf());
        }
        entries.push((relative, entry.is_dir()));
    }
    ensure!(!entries.is_empty(), "toy ZIP is empty: {}", archive_path.display());
    for (relative, _) in &entries {
        for ancestor in relative.ancestors().skip(1) {
            ensure!(
                explicit.get(ancestor).is_none_or(|is_dir| *is_dir),
                "ZIP file is also a parent directory: {}",
                ancestor.display()
            );
        }
    }
    // A previous extraction may have been interrupted, but untracked files cannot be trusted.
    for entry in WalkDir::new(root).follow_links(false).min_depth(1) {
        let entry = entry.with_context(|| format!("walking {}", root.display()))?;
        let relative = entry.path().strip_prefix(root)?;
        ensure!(
            known.contains(relative),
            "untracked file or directory in toy dataset: {}",
            entry.path().display()
        );
        ensure!(
            entry.file_type().is_dir() || entry.file_type().is_file(),
            "unsafe dataset file type: {}",
            entry.path().display()
        );
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
                "extracted toy file differs from verified ZIP; refusing to overwrite: {}",
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
    ensure_directory(root, target.parent().context("LOTUS destination has no parent")?)?;
    let mut decoded = GzDecoder::new(BufReader::new(File::open(compressed)?));
    if regular_file(target)? {
        let mut existing = BufReader::new(File::open(target)?);
        ensure!(
            streams_equal(&mut decoded, &mut existing)?,
            "existing LOTUS CSV differs from verified gzip; refusing to replace {}",
            target.display()
        );
    } else {
        let mut temporary = NamedTempFile::new_in(target.parent().unwrap())?;
        io::copy(&mut decoded, &mut temporary)
            .with_context(|| format!("decompressing LOTUS CSV from {}", compressed.display()))?;
        temporary.as_file_mut().sync_all()?;
        temporary.persist_noclobber(target).with_context(|| {
            format!("publishing LOTUS CSV without replacing {}", target.display())
        })?;
    }
    Ok(())
}
