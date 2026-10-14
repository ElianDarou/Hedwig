use anyhow::{Context, bail};
use clap::Parser;
use fuser::{
    Config, Errno, FileAttr, FileHandle, FileType, Filesystem, FopenFlags, INodeNo, LockOwner,
    MountOption, OpenFlags, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry, ReplyOpen, Request,
};
use hedwig_client::{CloudClient, RemoteFile};
use hedwig_core::CloudPath;
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    ffi::OsStr,
    fs::OpenOptions,
    io::Write,
    path::PathBuf,
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, UNIX_EPOCH},
};
use tokio::runtime::Runtime;

const TTL: Duration = Duration::from_secs(2);

#[derive(Debug, Parser)]
#[command(version, about = "Read-only on-demand Hedwig mount for Linux")]
struct Arguments {
    #[arg(long, env = "HEDWIG_SERVER")]
    server: String,
    #[arg(long, env = "HEDWIG_TOKEN", hide_env_values = true)]
    token: String,
    #[arg(long, env = "HEDWIG_SPACE")]
    space: String,
    #[arg(long)]
    mountpoint: PathBuf,
    #[arg(long)]
    cache_dir: PathBuf,
    #[arg(long, default_value_t = 15)]
    refresh_seconds: u64,
}

pub struct MountConfig {
    pub server: String,
    pub token: String,
    pub space: String,
    pub mountpoint: PathBuf,
    pub cache_dir: PathBuf,
    pub refresh_seconds: u64,
}

pub struct MountHandle {
    session: Option<fuser::BackgroundSession>,
    stop_refresh: Arc<AtomicBool>,
}

impl MountHandle {
    pub fn unmount(mut self) -> std::io::Result<()> {
        self.stop_refresh.store(true, Ordering::Relaxed);
        self.session.take().unwrap().umount_and_join()
    }

    fn wait(mut self) -> std::io::Result<()> {
        self.session.take().unwrap().join()
    }
}

impl Drop for MountHandle {
    fn drop(&mut self) {
        self.stop_refresh.store(true, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone)]
struct Node {
    inode: u64,
    parent: u64,
    name: String,
    path: String,
    kind: FileType,
    size: u64,
    file_id: Option<String>,
    version_id: Option<String>,
}

#[derive(Debug, Default)]
struct Tree {
    nodes: HashMap<u64, Node>,
    children: HashMap<u64, Vec<u64>>,
    by_parent_name: HashMap<(u64, String), u64>,
}

enum PendingNode {
    Directory,
    File(RemoteFile),
}

impl Tree {
    fn from_files(files: Vec<RemoteFile>) -> anyhow::Result<Self> {
        let mut paths = BTreeMap::<String, PendingNode>::new();
        for file in files {
            if file.deleted_at.is_some() || file.current_version_id.is_none() {
                continue;
            }
            let path = CloudPath::parse(&file.path)?.as_str().to_owned();
            let components: Vec<_> = path.split('/').collect();
            for end in 1..components.len() {
                let directory = components[..end].join("/");
                if matches!(paths.get(&directory), Some(PendingNode::File(_))) {
                    bail!("Datei und Ordner kollidieren bei {directory}");
                }
                paths.entry(directory).or_insert(PendingNode::Directory);
            }
            if matches!(paths.get(&path), Some(PendingNode::Directory)) {
                bail!("Datei und Ordner kollidieren bei {path}");
            }
            paths.insert(path, PendingNode::File(file));
        }

        let mut tree = Tree::default();
        tree.nodes.insert(
            1,
            Node {
                inode: 1,
                parent: 1,
                name: String::new(),
                path: String::new(),
                kind: FileType::Directory,
                size: 0,
                file_id: None,
                version_id: None,
            },
        );
        let mut inode_by_path = HashMap::from([(String::new(), 1_u64)]);
        let mut used = HashSet::from([1_u64]);
        for (path, pending) in paths {
            let parent_path = path.rsplit_once('/').map_or("", |(parent, _)| parent);
            let parent = *inode_by_path
                .get(parent_path)
                .with_context(|| format!("Übergeordneter Ordner für {path} fehlt"))?;
            let name = path.rsplit('/').next().unwrap().to_owned();
            let inode = stable_inode(&path, &mut used);
            let node = match pending {
                PendingNode::Directory => Node {
                    inode,
                    parent,
                    name,
                    path: path.clone(),
                    kind: FileType::Directory,
                    size: 0,
                    file_id: None,
                    version_id: None,
                },
                PendingNode::File(file) => Node {
                    inode,
                    parent,
                    name,
                    path: path.clone(),
                    kind: FileType::RegularFile,
                    size: file.size.try_into().context("Negative Dateigröße")?,
                    file_id: Some(file.id),
                    version_id: file.current_version_id,
                },
            };
            tree.by_parent_name
                .insert((parent, node.name.clone()), inode);
            tree.children.entry(parent).or_default().push(inode);
            inode_by_path.insert(path, inode);
            tree.nodes.insert(inode, node);
        }
        for children in tree.children.values_mut() {
            children.sort_by(|left, right| {
                tree.nodes[left]
                    .name
                    .to_lowercase()
                    .cmp(&tree.nodes[right].name.to_lowercase())
            });
        }
        Ok(tree)
    }
}

fn stable_inode(path: &str, used: &mut HashSet<u64>) -> u64 {
    let hash = blake3::hash(path.as_bytes());
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&hash.as_bytes()[..8]);
    let mut inode = u64::from_le_bytes(bytes).max(2);
    while !used.insert(inode) {
        inode = inode.wrapping_add(1).max(2);
    }
    inode
}

struct CloudFilesystem {
    client: CloudClient,
    runtime: Arc<Runtime>,
    tree: Arc<RwLock<Tree>>,
    cache_dir: PathBuf,
    download_gate: Mutex<()>,
    uid: u32,
    gid: u32,
}

impl CloudFilesystem {
    fn attr(&self, node: &Node) -> FileAttr {
        FileAttr {
            ino: INodeNo(node.inode),
            size: node.size,
            blocks: node.size.div_ceil(512),
            atime: UNIX_EPOCH,
            mtime: UNIX_EPOCH,
            ctime: UNIX_EPOCH,
            crtime: UNIX_EPOCH,
            kind: node.kind,
            perm: if node.kind == FileType::Directory {
                0o555
            } else {
                0o444
            },
            nlink: if node.kind == FileType::Directory {
                2
            } else {
                1
            },
            uid: self.uid,
            gid: self.gid,
            rdev: 0,
            flags: 0,
            blksize: 4096,
        }
    }

    fn cached_content(&self, node: &Node) -> Result<Vec<u8>, Errno> {
        let file_id = node.file_id.as_deref().ok_or(Errno::EISDIR)?;
        let version_id = node.version_id.as_deref().ok_or(Errno::ESTALE)?;
        let cache_name = blake3::hash(version_id.as_bytes()).to_hex().to_string();
        let target = self.cache_dir.join(&cache_name);
        if let Ok(data) = std::fs::read(&target) {
            return Ok(data);
        }
        let _gate = self.download_gate.lock().map_err(|_| Errno::EIO)?;
        if let Ok(data) = std::fs::read(&target) {
            return Ok(data);
        }
        let (manifest, data) = self
            .runtime
            .block_on(self.client.download_verified(file_id))
            .map_err(|error| {
                eprintln!("Cloud-Datei {} ist nicht verfügbar: {error}", node.path);
                Errno::EHOSTUNREACH
            })?;
        if manifest.version_id != version_id || manifest.file_id != file_id {
            eprintln!(
                "Cloud-Datei {} wurde während des Öffnens geändert",
                node.path
            );
            return Err(Errno::ESTALE);
        }
        std::fs::create_dir_all(&self.cache_dir).map_err(|_| Errno::EIO)?;
        let temporary = self.cache_dir.join(format!(".{cache_name}.partial"));
        let mut output = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)
            .map_err(|_| Errno::EIO)?;
        output.write_all(&data).map_err(|_| Errno::EIO)?;
        output.sync_all().map_err(|_| Errno::EIO)?;
        std::fs::rename(&temporary, &target).map_err(|_| Errno::EIO)?;
        Ok(data)
    }
}

impl Filesystem for CloudFilesystem {
    fn lookup(&self, _request: &Request, parent: INodeNo, name: &OsStr, reply: ReplyEntry) {
        let Some(name) = name.to_str() else {
            reply.error(Errno::ENOENT);
            return;
        };
        let tree = self.tree.read().unwrap();
        let Some(inode) = tree.by_parent_name.get(&(parent.0, name.to_owned())) else {
            reply.error(Errno::ENOENT);
            return;
        };
        reply.entry(&TTL, &self.attr(&tree.nodes[inode]), fuser::Generation(0));
    }

    fn getattr(
        &self,
        _request: &Request,
        inode: INodeNo,
        _handle: Option<FileHandle>,
        reply: ReplyAttr,
    ) {
        let tree = self.tree.read().unwrap();
        match tree.nodes.get(&inode.0) {
            Some(node) => reply.attr(&TTL, &self.attr(node)),
            None => reply.error(Errno::ENOENT),
        }
    }

    fn open(&self, _request: &Request, inode: INodeNo, _flags: OpenFlags, reply: ReplyOpen) {
        let tree = self.tree.read().unwrap();
        match tree.nodes.get(&inode.0) {
            Some(node) if node.kind == FileType::RegularFile => {
                reply.opened(FileHandle(0), FopenFlags::empty())
            }
            Some(_) => reply.error(Errno::EISDIR),
            None => reply.error(Errno::ENOENT),
        }
    }

    fn read(
        &self,
        _request: &Request,
        inode: INodeNo,
        _handle: FileHandle,
        offset: u64,
        size: u32,
        _flags: OpenFlags,
        _lock_owner: Option<LockOwner>,
        reply: ReplyData,
    ) {
        let node = {
            let tree = self.tree.read().unwrap();
            tree.nodes.get(&inode.0).cloned()
        };
        let Some(node) = node else {
            reply.error(Errno::ENOENT);
            return;
        };
        let data = match self.cached_content(&node) {
            Ok(data) => data,
            Err(error) => {
                reply.error(error);
                return;
            }
        };
        let start = (offset as usize).min(data.len());
        let end = start.saturating_add(size as usize).min(data.len());
        reply.data(&data[start..end]);
    }

    fn readdir(
        &self,
        _request: &Request,
        inode: INodeNo,
        _handle: FileHandle,
        offset: u64,
        mut reply: ReplyDirectory,
    ) {
        let tree = self.tree.read().unwrap();
        let Some(directory) = tree.nodes.get(&inode.0) else {
            reply.error(Errno::ENOENT);
            return;
        };
        if directory.kind != FileType::Directory {
            reply.error(Errno::ENOTDIR);
            return;
        }
        let mut entries = vec![
            (directory.inode, FileType::Directory, ".".to_owned()),
            (directory.parent, FileType::Directory, "..".to_owned()),
        ];
        entries.extend(
            tree.children
                .get(&inode.0)
                .into_iter()
                .flatten()
                .map(|child| {
                    let node = &tree.nodes[child];
                    (node.inode, node.kind, node.name.clone())
                }),
        );
        for (index, (child, kind, name)) in entries.into_iter().enumerate().skip(offset as usize) {
            if reply.add(INodeNo(child), (index + 1) as u64, kind, name) {
                break;
            }
        }
        reply.ok();
    }
}

pub fn run() -> anyhow::Result<()> {
    for (current, legacy) in [
        ("HEDWIG_SERVER", "CLOUD_SERVER"),
        ("HEDWIG_TOKEN", "CLOUD_TOKEN"),
        ("HEDWIG_SPACE", "CLOUD_SPACE"),
    ] {
        if std::env::var_os(current).is_none()
            && let Some(value) = std::env::var_os(legacy)
        {
            // SAFETY: the mount process has not created worker threads yet.
            unsafe { std::env::set_var(current, value) };
        }
    }
    let arguments = Arguments::parse();
    let mountpoint = arguments.mountpoint.clone();
    start_mount(MountConfig {
        server: arguments.server,
        token: arguments.token,
        space: arguments.space,
        mountpoint: arguments.mountpoint,
        cache_dir: arguments.cache_dir,
        refresh_seconds: arguments.refresh_seconds,
    })?
    .wait()
    .with_context(|| format!("Cloud-Mount {} wurde beendet", mountpoint.display()))?;
    Ok(())
}

pub fn start_mount(arguments: MountConfig) -> anyhow::Result<MountHandle> {
    if !arguments.mountpoint.is_dir() {
        bail!(
            "Mountpunkt {} ist kein erreichbarer Ordner",
            arguments.mountpoint.display()
        );
    }
    std::fs::create_dir_all(&arguments.cache_dir)?;
    let runtime = Arc::new(Runtime::new()?);
    let client = CloudClient::new(&arguments.server, arguments.token)?;
    let files = runtime.block_on(client.files(&arguments.space, false))?;
    let tree = Arc::new(RwLock::new(Tree::from_files(files)?));

    let refresh_tree = Arc::clone(&tree);
    let refresh_runtime = Arc::clone(&runtime);
    let refresh_client = client.clone();
    let refresh_space = arguments.space.clone();
    let refresh_seconds = arguments.refresh_seconds.max(5);
    let stop_refresh = Arc::new(AtomicBool::new(false));
    let refresh_stop = Arc::clone(&stop_refresh);
    std::thread::spawn(move || {
        loop {
            for _ in 0..refresh_seconds {
                if refresh_stop.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(Duration::from_secs(1));
            }
            match refresh_runtime
                .block_on(refresh_client.files(&refresh_space, false))
                .map_err(anyhow::Error::from)
                .and_then(Tree::from_files)
            {
                Ok(next) => *refresh_tree.write().unwrap() = next,
                Err(error) => {
                    eprintln!("Cloud-Mount konnte die Dateiliste nicht aktualisieren: {error:#}")
                }
            }
        }
    });

    let filesystem = CloudFilesystem {
        client,
        runtime,
        tree,
        cache_dir: arguments.cache_dir,
        download_gate: Mutex::new(()),
        uid: unsafe { libc::getuid() },
        gid: unsafe { libc::getgid() },
    };
    let mut config = Config::default();
    config.mount_options.extend([
        MountOption::RO,
        MountOption::NoExec,
        MountOption::NoSuid,
        MountOption::NoDev,
        MountOption::DefaultPermissions,
        MountOption::FSName("hedwig".into()),
    ]);
    let session =
        fuser::spawn_mount(filesystem, &arguments.mountpoint, &config).with_context(|| {
            format!(
                "Cloud-Mount {} konnte nicht gestartet werden",
                arguments.mountpoint.display()
            )
        })?;
    Ok(MountHandle {
        session: Some(session),
        stop_refresh,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hedwig_server::{AppConfig, build_app};
    use serde_json::{Value, json};
    use tempfile::TempDir;
    use tokio::net::TcpListener;

    fn remote(path: &str, id: &str, size: i64) -> RemoteFile {
        RemoteFile {
            id: id.into(),
            path: path.into(),
            size,
            current_version_id: Some(format!("version-{id}")),
            updated_at: String::new(),
            deleted_at: None,
        }
    }

    #[test]
    fn creates_stable_read_only_tree() {
        let first = Tree::from_files(vec![
            remote("Projekte/Plan.md", "1", 42),
            remote("Fotos/Urlaub/Bild.jpg", "2", 900),
        ])
        .unwrap();
        let second = Tree::from_files(vec![
            remote("Fotos/Urlaub/Bild.jpg", "2", 900),
            remote("Projekte/Plan.md", "1", 42),
        ])
        .unwrap();
        let first_paths: HashMap<_, _> = first
            .nodes
            .values()
            .map(|node| (&node.path, node.inode))
            .collect();
        let second_paths: HashMap<_, _> = second
            .nodes
            .values()
            .map(|node| (&node.path, node.inode))
            .collect();
        assert_eq!(first_paths, second_paths);
        assert!(
            first
                .nodes
                .values()
                .any(|node| node.path == "Fotos/Urlaub" && node.kind == FileType::Directory)
        );
        assert!(
            first
                .nodes
                .values()
                .any(|node| node.path == "Projekte/Plan.md" && node.size == 42)
        );
    }

    #[test]
    fn rejects_file_directory_collision() {
        let error = Tree::from_files(vec![remote("a", "1", 1), remote("a/b", "2", 1)]).unwrap_err();
        assert!(error.to_string().contains("kollidieren"));
    }

    #[test]
    #[ignore = "requires a usable /dev/fuse on a real Linux host"]
    fn actual_mount_loads_on_demand_and_reports_network_failure() {
        let runtime = Arc::new(Runtime::new().unwrap());
        let server_data = TempDir::new().unwrap();
        let (base, server_task) = runtime.block_on(async {
            let app = build_app(AppConfig::new(server_data.path().to_path_buf()))
                .await
                .unwrap();
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}/", listener.local_addr().unwrap());
            let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            (base, task)
        });
        let setup: Value = runtime.block_on(async {
            reqwest::Client::new()
                .post(format!("{base}v1/setup"))
                .json(&json!({
                    "username":"mount-test",
                    "password":"correct horse battery staple",
                    "device_name":"Linux mount test"
                }))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap()
        });
        let token = setup["token"].as_str().unwrap().to_owned();
        let space = setup["personal_space_id"].as_str().unwrap().to_owned();
        let client = CloudClient::new(&base, token).unwrap();
        runtime
            .block_on(client.upload_bytes(
                &space,
                "Dokumente/Geladen.txt",
                None,
                None,
                b"echter Inhalt",
                "mount-upload-1",
            ))
            .unwrap();
        runtime
            .block_on(client.upload_bytes(
                &space,
                "Dokumente/NichtImCache.txt",
                None,
                None,
                b"darf nicht leer erscheinen",
                "mount-upload-2",
            ))
            .unwrap();
        let files = runtime.block_on(client.files(&space, false)).unwrap();
        let tree = Arc::new(RwLock::new(Tree::from_files(files).unwrap()));
        let mountpoint = TempDir::new().unwrap();
        let cache = TempDir::new().unwrap();
        let filesystem = CloudFilesystem {
            client,
            runtime: Arc::clone(&runtime),
            tree,
            cache_dir: cache.path().to_path_buf(),
            download_gate: Mutex::new(()),
            uid: unsafe { libc::getuid() },
            gid: unsafe { libc::getgid() },
        };
        let mut config = Config::default();
        config.mount_options.extend([
            MountOption::RO,
            MountOption::NoExec,
            MountOption::DefaultPermissions,
            MountOption::FSName("hedwig-test".into()),
        ]);
        let session = fuser::spawn_mount(filesystem, mountpoint.path(), &config).unwrap();

        let cached_path = mountpoint.path().join("Dokumente/Geladen.txt");
        assert_eq!(std::fs::read(&cached_path).unwrap(), b"echter Inhalt");
        assert_eq!(std::fs::read_dir(cache.path()).unwrap().count(), 1);
        server_task.abort();
        runtime.block_on(async { tokio::task::yield_now().await });

        assert_eq!(
            std::fs::read(&cached_path).unwrap(),
            b"echter Inhalt",
            "Verifizierter Cache muss offline lesbar bleiben"
        );
        let missing = std::fs::read(mountpoint.path().join("Dokumente/NichtImCache.txt"));
        assert!(
            missing.is_err(),
            "Nicht geladene Datei darf nie leer erscheinen"
        );
        drop(session);
    }
}
