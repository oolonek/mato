use std::fs::{self, File};
use std::io::ErrorKind;
use std::io::Write;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, UNIX_EPOCH};

use anyhow::{Context, Result, bail, ensure};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};

use crate::config::{Config, QleverSettings};
use crate::job;

const UI_CONTAINER: &str = "mato-qlever-ui";
const PREFIXES: &str = "@prefix kg: <urn:mato:term:> .\n@prefix prov: <http://www.w3.org/ns/prov#> .\n@prefix wd: <http://www.wikidata.org/entity/> .\n@prefix wdt: <http://www.wikidata.org/prop/direct/> .\n";

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Backend {
    name: String,
    graph: PathBuf,
    graph_size: u64,
    graph_modified: u64,
    index: PathBuf,
    port: u16,
    engine_image: String,
}
#[derive(Eq, PartialEq, Serialize, Deserialize)]
struct IndexSource {
    graph: PathBuf,
    graph_size: u64,
    graph_modified: u64,
    engine_image: String,
}


fn command(command: &mut Command, description: &str) -> Result<()> {
    let status = command.status().with_context(|| format!("starting {description}"))?;
    ensure!(status.success(), "{description} failed ({status})");
    Ok(())
}

fn output(command: &mut Command, description: &str) -> Result<String> {
    let result = command.output().with_context(|| format!("starting {description}"))?;
    ensure!(result.status.success(), "{description}: {}", String::from_utf8_lossy(&result.stderr));
    String::from_utf8(result.stdout).with_context(|| format!("non-UTF-8 {description} output"))
}

fn user_id(flag: &str) -> Result<String> {
    Ok(output(Command::new("id").arg(flag), "reading server account ID")?.trim().to_owned())
}

fn docker_user() -> Result<String> {
    Ok(format!("{}:{}", user_id("-u")?, user_id("-g")?))
}

fn mount(path: &Path, target: &str, readonly: bool) -> String {
    format!("type=bind,src={},dst={target}{}", path.display(), if readonly { ",readonly" } else { "" })
}

fn docker_container(name: &str) -> Result<Option<serde_json::Value>> {
    let result = Command::new("docker").args(["container", "inspect", name]).output()
        .with_context(|| format!("inspecting Docker container {name}"))?;
    if !result.status.success() {
        let error = String::from_utf8_lossy(&result.stderr);
        if error.contains("No such container") || error.contains("No such object") {
            return Ok(None);
        }
        bail!("unable to inspect Docker container {name}: {error}");
    }
    let mut containers: Vec<serde_json::Value> = serde_json::from_slice(&result.stdout)?;
    Ok(containers.pop())
}

fn ledger_path(settings: &QleverSettings) -> PathBuf {
    settings.ui_data.join("mato-backends.json")
}

fn ledger(settings: &QleverSettings) -> Result<Vec<Backend>> {
    let path = ledger_path(settings);
    match fs::symlink_metadata(&path) {
        Ok(metadata) => ensure!(metadata.file_type().is_file(),
            "backend ledger is not a regular file: {}", path.display()),
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).with_context(|| format!("inspecting backend ledger {}", path.display())),
    }
    serde_json::from_reader(File::open(&path)?).with_context(|| format!("reading Mato backend ledger {}", path.display()))
}

fn save_ledger(settings: &QleverSettings, backends: &[Backend]) -> Result<()> {
    let path = ledger_path(settings);
    let mut temporary = tempfile::NamedTempFile::new_in(&settings.ui_data)?;
    serde_json::to_writer_pretty(&mut temporary, backends)?;
    temporary.write_all(b"\n")?;
    temporary.as_file_mut().sync_all()?;
    temporary.persist(&path).with_context(|| format!("saving backend ledger {}", path.display()))?;
    Ok(())
}

fn free_port(port: u16, description: &str) -> Result<()> {
    ensure!(port >= 1024, "{description} port must be at least 1024");
    TcpListener::bind(("127.0.0.1", port))
        .with_context(|| format!("{description} loopback port {port} is occupied"))?;
    Ok(())
}

fn wait_for(url: &str, description: &str) -> Result<()> {
    let client = Client::builder().timeout(Duration::from_secs(2)).build()?;
    for _ in 0..60 {
        if client.get(url).send().is_ok_and(|response| response.status().is_success()) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    bail!("{description} did not respond at {url}; inspect its Docker logs")
}

fn graph_identity(graph: &Path) -> Result<(PathBuf, u64, u64)> {
    let graph = graph.canonicalize().with_context(|| format!("KG graph missing: {}", graph.display()))?;
    let metadata = fs::metadata(&graph)?;
    ensure!(metadata.is_file() && metadata.len() > 0, "KG graph must be a nonempty file: {}", graph.display());
    ensure!(graph.extension().is_some_and(|extension| extension == "nq"), "QLever helper expects a Peregrino .nq graph");
    let modified = metadata.modified()?.duration_since(UNIX_EPOCH)?.as_nanos();
    let modified = u64::try_from(modified).context("graph modification time exceeds u64 nanoseconds")?;
    Ok((graph, metadata.len(), modified))
}

fn validate_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty() && name.len() <= 60
            && name.bytes().all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
            && !name.starts_with('-') && !name.ends_with('-'),
        "QLever backend name must be 1–60 lowercase ASCII letters, digits or internal hyphens"
    );
    Ok(())
}

fn index(source: &IndexSource, directory: &Path, user: &str) -> Result<()> {
    if directory.exists() {
        let receipt = directory.join("mato-index-source.json");
        let saved: IndexSource = serde_json::from_reader(File::open(&receipt)
            .with_context(|| format!("index directory exists without Mato receipt; inspect before reuse: {}", directory.display()))?)?;
        ensure!(&saved == source && directory.join("index.meta-data.json").is_file(),
            "index directory belongs to another graph or has an incomplete build; choose another name or inspect {}",
            directory.display());
        return Ok(());
    }
    fs::create_dir(directory).with_context(|| format!("creating new QLever index directory {}", directory.display()))?;
    let receipt = directory.join("mato-index-source.json");
    let mut source_file = fs::OpenOptions::new().write(true).create_new(true).open(&receipt)?;
    serde_json::to_writer_pretty(&mut source_file, source)?;
    source_file.sync_all()?;
    let settings = directory.join("index.settings.json");
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&settings)?;
    file.write_all(br#"{"ascii-prefixes-only":false,"num-triples-per-batch":100000}"#)?;
    file.sync_all()?;
    let mut docker = Command::new("docker");
    docker.args(["run", "--rm", "--init", "--user", user, "--mount"])
        .arg(mount(directory, "/index", false))
        .arg("--mount").arg(mount(&source.graph, "/source/graph.nq", true))
        .args(["--workdir", "/index", "--entrypoint", "IndexBuilderMain", &source.engine_image,
            "-i", "index", "-s", "/index/index.settings.json",
            "--vocabulary-type", "on-disk-compressed", "-F", "nq", "-f", "/source/graph.nq"]);
    command(&mut docker, "building QLever index from read-only graph")
        .with_context(|| format!("index build did not complete; inspect (do not reuse) {}", directory.display()))?;
    ensure!(directory.join("index.meta-data.json").is_file(),
        "QLever produced no index metadata in {}", directory.display());
    Ok(())
}

fn start_engine(name: &str, directory: &Path, image: &str, port: u16, user: &str) -> Result<()> {
    let container = format!("mato-qlever-{name}");
    if let Some(existing) = docker_container(&container)? {
        let existing_image = existing.pointer("/Config/Image").and_then(serde_json::Value::as_str);
        let mount_matches = existing.get("Mounts").and_then(serde_json::Value::as_array)
            .is_some_and(|mounts| mounts.iter().any(|entry| {
                entry.get("Source").and_then(serde_json::Value::as_str) == directory.to_str()
                    && entry.get("Destination").and_then(serde_json::Value::as_str) == Some("/index")
                    && entry.get("RW").and_then(serde_json::Value::as_bool) == Some(false)
            }));
        let published = existing.pointer(&format!("/HostConfig/PortBindings/{port}~1tcp/0"));
        let port_string = port.to_string();
        ensure!(existing_image == Some(image) && mount_matches
            && published.and_then(|binding| binding.get("HostIp")).and_then(serde_json::Value::as_str) == Some("127.0.0.1")
            && published.and_then(|binding| binding.get("HostPort")).and_then(serde_json::Value::as_str) == Some(port_string.as_str()),
            "existing {container} does not use this loopback port, image and read-only index; refusing to modify it");
        if existing.pointer("/State/Running").and_then(serde_json::Value::as_bool) != Some(true) {
            command(Command::new("docker").args(["start", &container]), "restarting matching QLever endpoint")?;
        }
    } else {
        free_port(port, "QLever engine")?;
        let mut docker = Command::new("docker");
        docker.args(["run", "--detach", "--restart", "unless-stopped", "--name", &container, "--init",
            "--user", user, "--publish"])
            .arg(format!("127.0.0.1:{port}:{port}"))
            .arg("--mount").arg(mount(directory, "/index", true))
            .args(["--workdir", "/index", "--entrypoint", "ServerMain", image,
                "-i", "index", "-p"]).arg(port.to_string())
            .args(["-m", "2G", "-c", "512M", "-e", "128M", "-k", "1000", "-j", "4", "-s", "30s"]);
        command(&mut docker, "starting loopback-only QLever endpoint")?;
    }
    wait_for(&format!("http://127.0.0.1:{port}/ping"), &container)
}

fn ui_docker(settings: &QleverSettings, user: &str) -> Command {
    let mut docker = Command::new("docker");
    docker.args(["run", "--rm", "--user", user, "--mount"])
        .arg(mount(&settings.ui_data, "/app/db", false))
        .arg("--env").arg("QLEVERUI_DATABASE_URL=sqlite:////app/db/qleverui.sqlite3")
        .arg("--entrypoint").arg("python");
    docker
}

fn ensure_ui(settings: &QleverSettings, user: &str) -> Result<()> {
    fs::create_dir_all(&settings.ui_data)?;
    let db = settings.ui_data.join("qleverui.sqlite3");
    if !db.exists() {
        command(ui_docker(settings, user).arg(&settings.ui_image).args(["manage.py", "migrate", "--noinput"]), "initializing shared QLever UI database")?;
    } else {
        ensure!(db.symlink_metadata()?.file_type().is_file(), "shared UI database is not a regular file: {}", db.display());
    }
    if let Some(existing) = docker_container(UI_CONTAINER)? {
        let image = existing.pointer("/Config/Image").and_then(serde_json::Value::as_str);
        let network = existing.pointer("/HostConfig/NetworkMode").and_then(serde_json::Value::as_str);
        let matches_data = existing.get("Mounts").and_then(serde_json::Value::as_array)
            .is_some_and(|mounts| mounts.iter().any(|entry| {
                entry.get("Source").and_then(serde_json::Value::as_str) == settings.ui_data.to_str()
                    && entry.get("Destination").and_then(serde_json::Value::as_str) == Some("/app/db")
            }));
        ensure!(image == Some(settings.ui_image.as_str()) && network == Some("host") && matches_data,
            "existing {UI_CONTAINER} uses another image, network or database; refusing to modify it");
        if existing.pointer("/State/Running").and_then(serde_json::Value::as_bool) != Some(true) {
            command(Command::new("docker").args(["start", UI_CONTAINER]), "restarting shared QLever UI")?;
        }
    } else {
        free_port(settings.ui_port, "QLever UI")?;
        let mut docker = Command::new("docker");
        docker.args(["run", "--detach", "--restart", "unless-stopped", "--name", UI_CONTAINER,
            "--init", "--network", "host", "--user", user, "--mount"])
            .arg(mount(&settings.ui_data, "/app/db", false))
            .arg("--env").arg("QLEVERUI_DATABASE_URL=sqlite:////app/db/qleverui.sqlite3")
            .arg("--env").arg("QLEVERUI_ALLOWED_HOSTS=127.0.0.1,localhost")
            .arg("--env").arg(format!("QLEVERUI_CSRF_TRUSTED_ORIGINS=http://127.0.0.1:{},http://127.0.0.1:17044", settings.ui_port))
            .arg("--entrypoint").arg("gunicorn").arg(&settings.ui_image)
            .args(["--bind"]).arg(format!("127.0.0.1:{}", settings.ui_port))
            .args(["--workers", "2", "--limit-request-line", "10000", "qlever.wsgi:application"]);
        command(&mut docker, "starting shared QLever UI")?;
    }
    wait_for(&format!("http://127.0.0.1:{}/", settings.ui_port), UI_CONTAINER)
}

fn register_ui(settings: &QleverSettings, name: &str, port: u16, user: &str) -> Result<()> {
    let flock = Path::new(env!("CARGO_MANIFEST_DIR")).parent().context("Mato has no flock root")?;
    let examples = [
        ("Graph coverage", "coverage.rq"),
        ("Ranked Wikidata-linked candidate evidence", "toy-candidates.rq"),
        ("Taxonomic evidence", "taxonomic-evidence.rq"),
        ("Reviewed Leaf observation subjects", "leaf-observations.rq"),
    ].into_iter().enumerate().map(|(index, (label, file))| -> Result<serde_json::Value> {
        Ok(serde_json::json!({"name": label, "sort_key": index + 1,
            "query": fs::read_to_string(flock.join("peregrino/kg").join(file))?}))
    }).collect::<Result<Vec<_>>>()?;
    const REGISTER: &str = r#"import json, os
from django.db import transaction
from backend.models import Backend, Example
slug = os.environ['MATO_SLUG']
url = os.environ['MATO_URL']
with transaction.atomic():
    backend = Backend.objects.filter(slug=slug).first()
    if backend is not None:
        if backend.name != 'Mato ' + slug or backend.baseUrl != url:
            raise SystemExit('QLever UI backend belongs to another endpoint: ' + slug)
    else:
        backend = Backend.objects.create(
            slug=slug, name='Mato ' + slug, baseUrl=url,
            sortKey='1', isDefault=not Backend.objects.filter(isDefault=True).exists(),
            dynamicSuggestions=0, suggestedPrefixes=os.environ['MATO_PREFIXES'])
    for example in json.loads(os.environ['MATO_EXAMPLES']):
        current = Example.objects.filter(backend_id=backend.id, name=example['name']).first()
        if current is None:
            Example.objects.create(
                backend_id=backend.id, name=example['name'],
                sortKey=example['sort_key'], query=example['query'])
        elif current.query != example['query'] or current.sortKey != example['sort_key']:
            current.query = example['query']
            current.sortKey = example['sort_key']
            current.save(update_fields=['query', 'sortKey'])
"#;
    let mut docker = ui_docker(settings, user);
    docker.arg("--env").arg(format!("MATO_SLUG={name}"))
        .arg("--env").arg(format!("MATO_URL=http://127.0.0.1:{port}"))
        .arg("--env").arg(format!("MATO_PREFIXES={PREFIXES}"))
        .arg("--env").arg(format!("MATO_EXAMPLES={}", serde_json::to_string(&examples)?))
        .arg(&settings.ui_image).args(["manage.py", "shell", "-c", REGISTER]);
    command(&mut docker, "registering a separate backend in the shared QLever UI")
}

pub(crate) fn add(config: &Path, graph: &Path, name: &str, port: u16) -> Result<()> {
    validate_name(name)?;
    let settings = QleverSettings::load(config)?;
    ensure!(port >= 1024 && port != settings.ui_port, "QLever engine port must be >=1024 and differ from UI port {}", settings.ui_port);
    let (graph, graph_size, graph_modified) = graph_identity(graph)?;
    ensure!(!graph.starts_with(&settings.index_root) && !graph.starts_with(&settings.ui_data),
        "source graph cannot be inside QLever index or UI output directories");
    output(
        Command::new("docker").args(["image", "inspect", "--format", "{{.Id}}"]).arg(&settings.ui_image),
        "checking locally built QLever UI image (see Mato's QLever setup guide)",
    )?;
    fs::create_dir_all(&settings.index_root)?;
    fs::create_dir_all(&settings.ui_data)?;
    let lock = settings.ui_data.join(".mato-qlever.lock");
    let _lock = Lock::acquire(&lock)?;
    let mut backends = ledger(&settings)?;
    ensure!(
        backends.is_empty() || settings.ui_data.join("qleverui.sqlite3").is_file(),
        "shared QLever UI database is missing while backends are registered; restore its database before adding graphs"
    );
    if let Some(existing) = backends.iter().find(|backend| backend.name == name) {
        ensure!(existing.graph == graph && existing.graph_size == graph_size && existing.graph_modified == graph_modified
            && existing.port == port && existing.engine_image == settings.engine_image,
            "backend {name} already belongs to another graph, graph revision, port or image; choose a new name");
        ensure!(existing.index == settings.index_root.join(name), "registered index is outside the configured backend directory");
        let user = docker_user()?;
        let source = IndexSource { graph: graph.clone(), graph_size, graph_modified,
            engine_image: settings.engine_image.clone() };
        index(&source, &existing.index, &user)?;
        start_engine(name, &existing.index, &settings.engine_image, port, &user)?;
        ensure_ui(&settings, &user)?;
        register_ui(&settings, name, port, &user)?;
        wait_for(&format!("http://127.0.0.1:{}/{name}", settings.ui_port), "registered UI backend")?;
        println!("Existing indexed graph: {}", existing.index.display());
        println!("SPARQL: http://127.0.0.1:{port}/");
        println!("Shared UI: http://127.0.0.1:{}/{name}", settings.ui_port);
        return Ok(());
    }
    ensure!(backends.iter().all(|backend| backend.port != port), "another Mato QLever backend already uses port {port}");
    let directory = settings.index_root.join(name);
    let source = IndexSource { graph: graph.clone(), graph_size, graph_modified, engine_image: settings.engine_image.clone() };
    if !directory.exists() {
        ensure!(docker_container(&format!("mato-qlever-{name}"))?.is_none(), "Docker container mato-qlever-{name} already exists");
        free_port(port, "QLever engine")?;
    }
    let user = docker_user()?;
    index(&source, &directory, &user)?;
    let (_, verified_size, verified_modified) = graph_identity(&graph)?;
    ensure!(verified_size == graph_size && verified_modified == graph_modified,
        "source graph changed during indexing; choose a new graph and name");
    start_engine(name, &directory, &settings.engine_image, port, &user)?;
    ensure_ui(&settings, &user)?;
    register_ui(&settings, name, port, &user)?;
    backends.push(Backend { name: name.to_owned(), graph, graph_size, graph_modified,
        index: directory.clone(), port, engine_image: settings.engine_image.clone() });
    save_ledger(&settings, &backends)?;
    println!("Indexed graph: {}", directory.display());
    println!("SPARQL: http://127.0.0.1:{port}/");
    println!("Shared UI: http://127.0.0.1:{}/{name}", settings.ui_port);
    println!("Laptop: mato qlever access --config {} --ssh USER@REACHABLE-HOST", config.display());
    Ok(())
}

// The OS releases this lock even if an index build is interrupted.
struct Lock { _file: File }

impl Lock {
    fn acquire(path: &Path) -> Result<Self> {
        let file = fs::OpenOptions::new().read(true).write(true).create(true).open(path)?;
        file.try_lock().with_context(|| format!(
            "another QLever backend registration is active for {}", path.display()
        ))?;
        Ok(Self { _file: file })
    }
}

// Reset is a scoped operation on registered backends, never on the shared UI
// directory or the QLever index root. Keep the registration lock until the
// ledger reflects every completed backend removal.
fn existing_directory(path: &Path, description: &str) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            ensure!(metadata.file_type().is_dir(),
                "{description} is not a real directory: {}", path.display());
            Ok(true)
        }
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).with_context(|| format!("inspecting {description} {}", path.display())),
    }
}

fn check_reset_container(backend: &Backend) -> Result<bool> {
    let container = format!("mato-qlever-{}", backend.name);
    let Some(existing) = docker_container(&container)? else {
        // A previous reset may already have removed this engine.
        return Ok(false);
    };
    let port = backend.port.to_string();
    let published = existing.pointer(&format!("/HostConfig/PortBindings/{}~1tcp/0", backend.port));
    let mounts = existing.get("Mounts").and_then(serde_json::Value::as_array);
    ensure!(
        existing.get("Name").and_then(serde_json::Value::as_str) == Some(format!("/{container}").as_str())
            && existing.pointer("/Config/Image").and_then(serde_json::Value::as_str) == Some(backend.engine_image.as_str())
            && mounts.is_some_and(|mounts| {
                let mut indexes = mounts.iter().filter(|mount| {
                    mount.get("Destination").and_then(serde_json::Value::as_str) == Some("/index")
                });
                indexes.next().is_some_and(|mount| {
                    mount.get("Type").and_then(serde_json::Value::as_str) == Some("bind")
                        && mount.get("Source").and_then(serde_json::Value::as_str) == backend.index.to_str()
                        && mount.get("RW").and_then(serde_json::Value::as_bool) == Some(false)
                }) && indexes.next().is_none()
            })
            && published.and_then(|binding| binding.get("HostIp")).and_then(serde_json::Value::as_str) == Some("127.0.0.1")
            && published.and_then(|binding| binding.get("HostPort")).and_then(serde_json::Value::as_str) == Some(port.as_str()),
        "existing {container} does not match its registered image, read-only index mount and loopback port; refusing reset"
    );
    Ok(true)
}

fn check_reset_ui_container(settings: &QleverSettings) -> Result<()> {
    if let Some(existing) = docker_container(UI_CONTAINER)? {
        let mounts = existing.get("Mounts").and_then(serde_json::Value::as_array);
        ensure!(
            existing.get("Name").and_then(serde_json::Value::as_str) == Some("/mato-qlever-ui")
                && existing.pointer("/Config/Image").and_then(serde_json::Value::as_str) == Some(&settings.ui_image)
                && existing.pointer("/HostConfig/NetworkMode").and_then(serde_json::Value::as_str) == Some("host")
                && mounts.is_some_and(|mounts| {
                    let mut databases = mounts.iter().filter(|mount| {
                        mount.get("Destination").and_then(serde_json::Value::as_str) == Some("/app/db")
                    });
                    databases.next().is_some_and(|mount| {
                        mount.get("Type").and_then(serde_json::Value::as_str) == Some("bind")
                            && mount.get("Source").and_then(serde_json::Value::as_str) == settings.ui_data.to_str()
                            && mount.get("RW").and_then(serde_json::Value::as_bool) == Some(true)
                    }) && databases.next().is_none()
                }),
            "existing {UI_CONTAINER} uses another image, network or UI database; refusing reset"
        );
    }
    Ok(())
}

fn ui_reset(settings: &QleverSettings, backends: &[Backend], delete: bool) -> Result<()> {
    const VERIFY: &str = r#"import json, os
from backend.models import Backend
for item in json.loads(os.environ['MATO_BACKENDS']):
    backend = Backend.objects.filter(slug=item['name']).first()
    if backend is not None and (backend.name != 'Mato ' + item['name'] or backend.baseUrl != 'http://127.0.0.1:' + str(item['port'])):
        raise SystemExit('QLever UI backend belongs to another endpoint: ' + item['name'])
"#;
    const DELETE: &str = r#"import json, os
from django.db import transaction
from backend.models import Backend, Example
for item in json.loads(os.environ['MATO_BACKENDS']):
    with transaction.atomic():
        backend = Backend.objects.filter(slug=item['name']).first()
        if backend is None:
            continue
        if backend.name != 'Mato ' + item['name'] or backend.baseUrl != 'http://127.0.0.1:' + str(item['port']):
            raise SystemExit('QLever UI backend belongs to another endpoint: ' + item['name'])
        was_default = backend.isDefault
        Example.objects.filter(backend_id=backend.id).delete()
        backend.delete()
        if was_default and not Backend.objects.filter(isDefault=True).exists():
            next_backend = Backend.objects.order_by('id').first()
            if next_backend is not None:
                next_backend.isDefault = True
                next_backend.save(update_fields=['isDefault'])
"#;
    let selected = backends.iter().map(|backend| serde_json::json!({
        "name": backend.name, "port": backend.port
    })).collect::<Vec<_>>();
    let user = docker_user()?;
    let mut docker = ui_docker(settings, &user);
    docker.arg("--env").arg(format!("MATO_BACKENDS={}", serde_json::to_string(&selected)?))
        .arg(&settings.ui_image).args(["manage.py", "shell", "-c", if delete { DELETE } else { VERIFY }]);
    command(&mut docker, if delete { "removing selected QLever UI backend and examples" }
        else { "verifying selected QLever UI backends" })
}

fn check_configured_graph_dir(config_path: &Path, top_n: Option<usize>, graph_dir: &Path) -> Result<()> {
    // Config::load resolves symlinks for comparison; reset must additionally
    // reject a configured output path redirected to a different job.
    let text = fs::read_to_string(config_path)?;
    let file: toml::Value = toml::from_str(&text)?;
    let workspace = file.get("workspace").and_then(toml::Value::as_str)
        .context("job config is missing workspace")?;
    let output = file.get("knowledge_graph").and_then(|section| section.get("output_dir"))
        .and_then(toml::Value::as_str).context("job config is missing knowledge_graph.output_dir")?;
    let workspace = Path::new(workspace);
    let workspace = if workspace.is_absolute() {
        workspace.to_path_buf()
    } else {
        config_path.parent().context("job config has no parent")?.join(workspace)
    }.canonicalize()?;
    let output = Path::new(output);
    let mut configured = if output.is_absolute() {
        output.to_path_buf()
    } else {
        workspace.join(output)
    };
    if let Some(n) = top_n {
        configured.push(n.to_string());
    }
    let mut traversed = PathBuf::new();
    for component in configured.components() {
        match component {
            std::path::Component::CurDir => continue,
            std::path::Component::ParentDir => { traversed.pop(); continue; }
            other => traversed.push(other.as_os_str()),
        }
        match fs::symlink_metadata(&traversed) {
            Ok(metadata) => ensure!(!metadata.file_type().is_symlink(),
                "knowledge graph output passes through a symlink: {}", traversed.display()),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error).with_context(|| format!("inspecting graph path {}", traversed.display())),
        }
    }
    ensure!(job::resolved_path(&configured)? == graph_dir,
        "configured knowledge graph path does not match the selected job: {}", configured.display());
    Ok(())
}

fn reset_targets(graph_dir: &Path, settings: &QleverSettings, backends: &[Backend]) -> Result<Vec<Backend>> {
    for shared in [&settings.index_root, &settings.ui_data] {
        ensure!(!graph_dir.starts_with(shared) && !shared.starts_with(graph_dir),
            "knowledge graph directory overlaps shared QLever data: {}", graph_dir.display());
    }
    existing_directory(graph_dir, "knowledge graph output")?;
    let selected: Vec<Backend> = backends.iter()
        .filter(|backend| backend.graph.starts_with(graph_dir) && backend.graph.as_path() != graph_dir)
        .cloned().collect();
    for backend in &selected {
        validate_name(&backend.name)?;
        ensure!(backend.index == settings.index_root.join(&backend.name)
                && backend.graph.is_absolute()
                && job::resolved_path(&backend.graph)? == backend.graph,
            "registered QLever backend {} has an unsafe source graph or index path", backend.name);
        match fs::symlink_metadata(&backend.graph) {
            Ok(metadata) => ensure!(!metadata.file_type().is_symlink(),
                "registered QLever graph is a symlink: {}", backend.graph.display()),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error).with_context(|| format!("inspecting graph {}", backend.graph.display())),
        }
        ensure!(backends.iter().filter(|other| other.name == backend.name || other.index == backend.index).count() == 1,
            "QLever backend {} shares an index or name with another registered backend", backend.name);
        if existing_directory(&backend.index, "QLever backend index")? {
            let receipt = backend.index.join("mato-index-source.json");
            ensure!(fs::symlink_metadata(&receipt)?.file_type().is_file(),
                "QLever index receipt is not a regular file: {}", receipt.display());
            let saved: IndexSource = serde_json::from_reader(File::open(&receipt)?)
                .with_context(|| format!("reading QLever source receipt {}", receipt.display()))?;
            ensure!(saved == IndexSource {
                graph: backend.graph.clone(), graph_size: backend.graph_size,
                graph_modified: backend.graph_modified, engine_image: backend.engine_image.clone()
            }, "QLever index {} belongs to another graph or image; refusing reset", backend.index.display());
        }
    }
    Ok(selected)
}

fn remove_registration(settings: &QleverSettings, name: &str) -> Result<()> {
    let remaining: Vec<Backend> = ledger(settings)?.into_iter()
        .filter(|entry| entry.name != name).collect();
    save_ledger(settings, &remaining)
}

pub(crate) fn reset_graph(config: &Config) -> Result<()> {
    let graph_dir = &config.graph_dir;
    ensure!(
        !graph_dir.starts_with(&config.dataset_root)
            && !graph_dir.starts_with(&config.flock_root)
            && !config.dataset_root.starts_with(graph_dir)
            && !config.flock_root.starts_with(graph_dir)
            && graph_dir != Path::new("/"),
        "unsafe knowledge graph directory: {}", graph_dir.display()
    );
    for source in [&config.path, &config.metadata, &config.ms_dir, &config.settings,
        &config.library, &config.lotus_csv, &config.taxon_index, &config.chemical_index] {
        ensure!(!job::resolved_path(source)?.starts_with(graph_dir),
            "knowledge graph output contains configured input, config or shared index: {}", source.display());
    }
    check_configured_graph_dir(&config.path, config.top_n, graph_dir)?;
    let settings = QleverSettings::load(&config.path)?;
    let graph_exists = existing_directory(graph_dir, "knowledge graph output")?;
    let ui_exists = existing_directory(&settings.ui_data, "shared QLever UI data")?;
    let _lock = if ui_exists {
        let path = settings.ui_data.join(".mato-qlever.lock");
        match fs::symlink_metadata(&path) {
            Ok(metadata) => ensure!(metadata.file_type().is_file(),
                "QLever registration lock is not a regular file: {}", path.display()),
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(error).with_context(|| format!("inspecting QLever lock {}", path.display())),
        }
        Some(Lock::acquire(&path)?)
    } else {
        None
    };
    let backends = if ui_exists { ledger(&settings)? } else { Vec::new() };
    let selected = reset_targets(graph_dir, &settings, &backends)?;
    // Validate every engine before modifying any of them.
    for backend in &selected {
        check_reset_container(backend)?;
    }
    if !selected.is_empty() {
        let ui_db = settings.ui_data.join("qleverui.sqlite3");
        ensure!(fs::symlink_metadata(&ui_db).is_ok_and(|metadata| metadata.file_type().is_file()),
            "shared QLever UI database is missing or unsafe: {}; restore it before reset", ui_db.display());
        check_reset_ui_container(&settings)?;
        // Check all names and endpoints before stopping even the first engine.
        ui_reset(&settings, &selected, false)?;
    }
    for backend in &selected {
        let container = format!("mato-qlever-{}", backend.name);
        if check_reset_container(backend)? {
            command(Command::new("docker").args(["rm", "--force", &container]),
                "removing selected QLever engine")
                .with_context(|| format!("reset stopped at {container}; retry with the same --kg option"))?;
        }
        if existing_directory(&backend.index, "QLever backend index")? {
            fs::remove_dir_all(&backend.index)
                .with_context(|| format!("removing QLever index {}; retry reset with --kg", backend.index.display()))?;
        }
        ui_reset(&settings, std::slice::from_ref(backend), true)
            .with_context(|| format!("removing UI backend {}; retry reset with --kg", backend.name))?;
        remove_registration(&settings, &backend.name)
            .with_context(|| format!("saving removal of backend {}; retry reset with --kg", backend.name))?;
        println!("Removed QLever engine, index and UI backend: {}", backend.name);
    }
    if graph_exists {
        fs::remove_dir_all(graph_dir)
            .with_context(|| format!("removing knowledge graph directory {}; retry reset with --kg", graph_dir.display()))?;
    }
    println!("Deleted knowledge graph output: {}", graph_dir.display());
    Ok(())
}

pub(crate) fn access(config: &Path, ssh_host: &str, local_ui_port: u16) -> Result<()> {
    ensure!(!ssh_host.is_empty() && ssh_host.bytes().all(|byte|
        byte.is_ascii_alphanumeric() || b".@_-".contains(&byte)), "SSH target must be a reachable user@host or SSH alias");
    ensure!(local_ui_port >= 1024, "laptop UI port must be >= 1024");
    let settings = QleverSettings::load(config)?;
    let mut backends = ledger(&settings)?;
    ensure!(!backends.is_empty(), "no Mato QLever backends registered; run mato qlever add first");
    backends.sort_unstable_by_key(|backend| backend.port);
    ensure!(backends.iter().all(|backend| backend.port != local_ui_port), "laptop UI port conflicts with a backend port");
    print!("ssh -N -o ExitOnForwardFailure=yes -L 127.0.0.1:{local_ui_port}:127.0.0.1:{}", settings.ui_port);
    for backend in &backends {
        print!(" -L 127.0.0.1:{0}:127.0.0.1:{0}", backend.port);
    }
    println!(" {ssh_host}");
    println!("On the laptop, open:");
    for backend in backends {
        println!("  http://127.0.0.1:{local_ui_port}/{}", backend.name);
    }
    println!("The laptop needs these API ports free; the browser calls each backend directly. Keep the SSH tunnel open.");
    Ok(())
}

#[cfg(test)]
mod reset_tests {
    use super::*;
    use std::os::unix::fs::symlink;

    fn settings(root: &Path) -> QleverSettings {
        QleverSettings {
            index_root: root.join("indexes"),
            ui_data: root.join("ui"),
            ui_image: "qleverui:test".to_owned(),
            engine_image: "qlever:test".to_owned(),
            ui_port: 7044,
        }
    }

    fn backend(settings: &QleverSettings, name: &str, graph: PathBuf, port: u16) -> Backend {
        Backend {
            name: name.to_owned(),
            graph,
            graph_size: 1,
            graph_modified: 123,
            index: settings.index_root.join(name),
            port,
            engine_image: settings.engine_image.clone(),
        }
    }

    #[test]
    fn configured_top_n_graph_path_rejects_symlink_and_wrong_job() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let config = root.join("job.toml");
        let real = root.join("graphs/real/50");
        fs::create_dir_all(&real).unwrap();
        fs::write(&config, format!(
            "workspace = \"{}\"\n[knowledge_graph]\noutput_dir = \"graphs/real\"\n",
            root.display()
        )).unwrap();
        check_configured_graph_dir(&config, Some(50), &real).unwrap();
        assert!(check_configured_graph_dir(&config, Some(50), &root.join("graphs/other/50"))
            .unwrap_err().to_string().contains("does not match the selected job"));

        symlink(root.join("graphs/real"), root.join("graphs/redirect")).unwrap();
        fs::write(&config, format!(
            "workspace = \"{}\"\n[knowledge_graph]\noutput_dir = \"graphs/redirect\"\n",
            root.display()
        )).unwrap();
        assert!(check_configured_graph_dir(&config, Some(50), &real)
            .unwrap_err().to_string().contains("passes through a symlink"));
    }

    #[test]
    fn reset_targets_only_select_graphs_inside_the_job_and_preserve_other_ledger_records() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let settings = settings(root);
        let graph_dir = root.join("graphs/job");
        let other_dir = root.join("graphs/other");
        fs::create_dir_all(&graph_dir).unwrap();
        fs::create_dir_all(&other_dir).unwrap();
        fs::create_dir_all(&settings.ui_data).unwrap();
        let mine = backend(&settings, "mine", graph_dir.join("mine.nq"), 7043);
        let other = backend(&settings, "other", other_dir.join("other.nq"), 7045);
        fs::create_dir_all(&other.index).unwrap();
        fs::write(other.index.join("sentinel"), b"keep").unwrap();
        fs::write(settings.ui_data.join("qleverui.sqlite3"), b"shared").unwrap();
        save_ledger(&settings, &[mine.clone(), other]).unwrap();

        let selected = reset_targets(&graph_dir, &settings, &ledger(&settings).unwrap()).unwrap();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].name, "mine");
        // An index already removed by a previous attempt must not prevent retry.
        assert!(!mine.index.exists());
        remove_registration(&settings, &mine.name).unwrap();
        let remaining = ledger(&settings).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].name, "other");
        assert!(other_dir.is_dir());
        assert_eq!(fs::read(remaining[0].index.join("sentinel")).unwrap(), b"keep");
        assert_eq!(fs::read(settings.ui_data.join("qleverui.sqlite3")).unwrap(), b"shared");
    }

    #[test]
    fn reset_refuses_redirected_graph_and_index_directories() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let settings = settings(root);
        let graph_dir = root.join("graphs/job");
        let redirected_graph = root.join("graphs/redirect");
        let external = root.join("external");
        fs::create_dir_all(&graph_dir).unwrap();
        fs::create_dir_all(&external).unwrap();
        symlink(&graph_dir, &redirected_graph).unwrap();
        assert!(reset_targets(&redirected_graph, &settings, &[]).unwrap_err()
            .to_string().contains("not a real directory"));

        fs::create_dir_all(&settings.index_root).unwrap();
        let mine = backend(&settings, "mine", graph_dir.join("mine.nq"), 7043);
        symlink(&external, &mine.index).unwrap();
        assert!(reset_targets(&graph_dir, &settings, &[mine]).unwrap_err()
            .to_string().contains("not a real directory"));
        assert!(external.is_dir());
    }

    #[test]
    fn reset_refuses_mismatched_index_path_receipt_and_redirected_graph_file() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path();
        let settings = settings(root);
        let graph_dir = root.join("graphs/job");
        fs::create_dir_all(&graph_dir).unwrap();
        let mut mine = backend(&settings, "mine", graph_dir.join("mine.nq"), 7043);
        mine.index = settings.index_root.join("other");
        assert!(reset_targets(&graph_dir, &settings, &[mine.clone()]).unwrap_err()
            .to_string().contains("unsafe source graph or index path"));

        mine.index = settings.index_root.join("mine");
        fs::create_dir_all(&mine.index).unwrap();
        let wrong = IndexSource {
            graph: root.join("graphs/other/other.nq"),
            graph_size: mine.graph_size,
            graph_modified: mine.graph_modified,
            engine_image: mine.engine_image.clone(),
        };
        fs::write(mine.index.join("mato-index-source.json"),
            serde_json::to_vec(&wrong).unwrap()).unwrap();
        assert!(reset_targets(&graph_dir, &settings, &[mine.clone()]).unwrap_err()
            .to_string().contains("belongs to another graph or image"));

        fs::remove_dir_all(&mine.index).unwrap();
        let mut escaping = mine.clone();
        escaping.graph = graph_dir.join("../other/outside.nq");
        assert!(reset_targets(&graph_dir, &settings, &[escaping]).unwrap_err()
            .to_string().contains("unsafe source graph or index path"));
        fs::write(root.join("external.nq"), b"x").unwrap();
        symlink(root.join("external.nq"), &mine.graph).unwrap();
        assert!(reset_targets(&graph_dir, &settings, &[mine]).unwrap_err()
            .to_string().contains("unsafe source graph or index path"));
    }
}
