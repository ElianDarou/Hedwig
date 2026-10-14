use anyhow::bail;
use hedwig_client::{CloudClient, RemoteFile};
use hedwig_core::CloudPath;
use std::{
    collections::BTreeSet,
    ffi::{OsStr, OsString, c_void},
    mem::{offset_of, size_of},
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    ptr::{null, null_mut},
    slice,
    sync::Arc,
};
use tokio::runtime::Runtime;
use windows_sys::{
    Win32::{
        Foundation::{HANDLE, STATUS_CLOUD_FILE_NETWORK_UNAVAILABLE, STATUS_SUCCESS},
        Storage::{
            CloudFilters::*,
            FileSystem::{FILE_ATTRIBUTE_NORMAL, FILE_BASIC_INFO},
        },
    },
    core::GUID,
};

const PROVIDER_ID: GUID = GUID {
    data1: 0x5361_97b1,
    data2: 0xe128,
    data3: 0x4d03,
    data4: [0x8d, 0x29, 0x98, 0x57, 0x6c, 0x41, 0x51, 0x70],
};

pub struct ProviderConfig {
    pub server: String,
    pub token: String,
    pub space_id: String,
    pub root: PathBuf,
}

struct CallbackContext {
    client: CloudClient,
    runtime: Arc<Runtime>,
    root: PathBuf,
    space_id: String,
}

pub struct ProviderSession {
    connection_key: CF_CONNECTION_KEY,
    root: PathBuf,
    _context: Box<Arc<CallbackContext>>,
}

unsafe impl Send for ProviderSession {}

impl ProviderSession {
    pub fn start(config: ProviderConfig) -> anyhow::Result<Self> {
        if !config.root.is_dir() {
            bail!(
                "Files-on-Demand-Stamm {} ist kein Ordner",
                config.root.display()
            );
        }
        let runtime = Arc::new(Runtime::new()?);
        let client = CloudClient::new(&config.server, config.token)?;
        let files = runtime.block_on(client.files(&config.space_id, false))?;
        register_root(&config.root, config.space_id.as_bytes())?;
        populate_placeholders(&config.root, files)?;

        let context = Box::new(Arc::new(CallbackContext {
            client,
            runtime,
            root: config.root.clone(),
            space_id: config.space_id,
        }));
        let callbacks = [
            CF_CALLBACK_REGISTRATION {
                Type: CF_CALLBACK_TYPE_FETCH_DATA,
                Callback: Some(fetch_data),
            },
            CF_CALLBACK_REGISTRATION {
                Type: CF_CALLBACK_TYPE_NOTIFY_FILE_CLOSE_COMPLETION,
                Callback: Some(file_closed),
            },
            CF_CALLBACK_REGISTRATION {
                Type: CF_CALLBACK_TYPE_NONE,
                Callback: None,
            },
        ];
        let mut connection_key = 0;
        let root = wide(&config.root);
        let result = unsafe {
            CfConnectSyncRoot(
                root.as_ptr(),
                callbacks.as_ptr(),
                (&*context as *const Arc<CallbackContext>).cast(),
                CF_CONNECT_FLAG_REQUIRE_FULL_FILE_PATH,
                &mut connection_key,
            )
        };
        check_hresult(result, "Files-on-Demand-Stamm verbinden")?;
        Ok(Self {
            connection_key,
            root: config.root,
            _context: context,
        })
    }

    pub fn unregister(mut self) -> anyhow::Result<()> {
        let result = unsafe { CfDisconnectSyncRoot(self.connection_key) };
        check_hresult(result, "Files-on-Demand-Verbindung trennen")?;
        self.connection_key = 0;
        Self::unregister_path(&self.root)
    }

    pub fn unregister_path(root: &Path) -> anyhow::Result<()> {
        let root = wide(root);
        check_hresult(
            unsafe { CfUnregisterSyncRoot(root.as_ptr()) },
            "Files-on-Demand-Stamm entfernen",
        )
    }

    pub fn keep_offline(path: &Path) -> anyhow::Result<()> {
        let handle = ProtectedHandle::open(path, CF_OPEN_FILE_FLAG_WRITE_ACCESS)?;
        check_hresult(
            unsafe {
                CfSetPinState(
                    handle.0,
                    CF_PIN_STATE_PINNED,
                    CF_SET_PIN_FLAG_NONE,
                    null_mut(),
                )
            },
            "Datei als offline verfügbar markieren",
        )?;
        check_hresult(
            unsafe { CfHydratePlaceholder(handle.0, 0, -1, CF_HYDRATE_FLAG_NONE, null_mut()) },
            "Datei vollständig herunterladen",
        )
    }

    pub fn free_space(path: &Path) -> anyhow::Result<()> {
        let handle = ProtectedHandle::open(path, CF_OPEN_FILE_FLAG_WRITE_ACCESS)?;
        let information = placeholder_information(handle.0)?;
        if information.InSyncState != CF_IN_SYNC_STATE_IN_SYNC {
            bail!("Lokale, noch nicht bestätigte Änderungen verhindern das Freigeben von Speicher");
        }
        check_hresult(
            unsafe {
                CfSetPinState(
                    handle.0,
                    CF_PIN_STATE_UNPINNED,
                    CF_SET_PIN_FLAG_NONE,
                    null_mut(),
                )
            },
            "Offline-Markierung aufheben",
        )?;
        check_hresult(
            unsafe { CfDehydratePlaceholder(handle.0, 0, -1, CF_DEHYDRATE_FLAG_NONE, null_mut()) },
            "Lokalen Dateiinhalt freigeben",
        )
    }
}

impl Drop for ProviderSession {
    fn drop(&mut self) {
        if self.connection_key != 0 {
            unsafe {
                CfDisconnectSyncRoot(self.connection_key);
            }
        }
    }
}

fn register_root(root: &Path, identity: &[u8]) -> anyhow::Result<()> {
    let root_wide = wide(root);
    let provider_name = wide(OsStr::new("Hedwig"));
    let provider_version = wide(OsStr::new(env!("CARGO_PKG_VERSION")));
    let registration = CF_SYNC_REGISTRATION {
        StructSize: size_of::<CF_SYNC_REGISTRATION>() as u32,
        ProviderName: provider_name.as_ptr(),
        ProviderVersion: provider_version.as_ptr(),
        SyncRootIdentity: identity.as_ptr().cast(),
        SyncRootIdentityLength: identity.len() as u32,
        FileIdentity: null(),
        FileIdentityLength: 0,
        ProviderId: PROVIDER_ID,
    };
    let policies = CF_SYNC_POLICIES {
        StructSize: size_of::<CF_SYNC_POLICIES>() as u32,
        Hydration: CF_HYDRATION_POLICY {
            Primary: CF_HYDRATION_POLICY_PROGRESSIVE,
            Modifier: CF_HYDRATION_POLICY_MODIFIER_AUTO_DEHYDRATION_ALLOWED,
        },
        Population: CF_POPULATION_POLICY {
            Primary: CF_POPULATION_POLICY_FULL,
            Modifier: CF_POPULATION_POLICY_MODIFIER_NONE,
        },
        InSync: CF_INSYNC_POLICY_TRACK_ALL,
        HardLink: CF_HARDLINK_POLICY_NONE,
        PlaceholderManagement: CF_PLACEHOLDER_MANAGEMENT_POLICY_DEFAULT,
    };
    let initial = unsafe {
        CfRegisterSyncRoot(
            root_wide.as_ptr(),
            &registration,
            &policies,
            CF_REGISTER_FLAG_MARK_IN_SYNC_ON_ROOT,
        )
    };
    if initial >= 0 {
        return Ok(());
    }
    check_hresult(
        unsafe {
            CfRegisterSyncRoot(
                root_wide.as_ptr(),
                &registration,
                &policies,
                CF_REGISTER_FLAG_UPDATE | CF_REGISTER_FLAG_MARK_IN_SYNC_ON_ROOT,
            )
        },
        "Files-on-Demand-Stamm registrieren oder aktualisieren",
    )
}

fn populate_placeholders(root: &Path, files: Vec<RemoteFile>) -> anyhow::Result<()> {
    let mut directories = BTreeSet::new();
    let mut current = Vec::new();
    for file in files {
        if file.deleted_at.is_some() || file.current_version_id.is_none() {
            continue;
        }
        let path = CloudPath::parse(&file.path)?;
        if let Some(parent) = Path::new(path.as_str()).parent() {
            directories.insert(parent.to_path_buf());
        }
        current.push((path.as_str().to_owned(), file));
    }
    for directory in directories {
        std::fs::create_dir_all(root.join(directory))?;
    }
    let root_wide = wide(root);
    for (path, file) in current {
        // A provider restart must reconnect to already materialized placeholders
        // instead of trying to create them a second time.
        if root.join(Path::new(&path)).exists() {
            continue;
        }
        let relative = wide(OsStr::new(&path.replace('/', "\\")));
        let identity = encode_identity(&file.id, file.current_version_id.as_deref().unwrap());
        let mut placeholder = CF_PLACEHOLDER_CREATE_INFO {
            RelativeFileName: relative.as_ptr(),
            FsMetadata: CF_FS_METADATA {
                BasicInfo: FILE_BASIC_INFO {
                    FileAttributes: FILE_ATTRIBUTE_NORMAL,
                    ..Default::default()
                },
                FileSize: file.size,
            },
            FileIdentity: identity.as_ptr().cast(),
            FileIdentityLength: identity.len() as u32,
            Flags: CF_PLACEHOLDER_CREATE_FLAG_MARK_IN_SYNC,
            ..Default::default()
        };
        let mut processed = 0;
        let result = unsafe {
            CfCreatePlaceholders(
                root_wide.as_ptr(),
                &mut placeholder,
                1,
                CF_CREATE_FLAG_STOP_ON_ERROR,
                &mut processed,
            )
        };
        if result < 0 && placeholder.Result < 0 {
            check_hresult(placeholder.Result, &format!("Platzhalter {path} erstellen"))?;
        }
        check_hresult(result, &format!("Platzhalter {path} erstellen"))?;
        if processed != 1 {
            bail!("Platzhalter {path} wurde nicht angelegt");
        }
    }
    Ok(())
}

unsafe extern "system" fn fetch_data(
    callback_info: *const CF_CALLBACK_INFO,
    callback_parameters: *const CF_CALLBACK_PARAMETERS,
) {
    unsafe {
        if callback_info.is_null() || callback_parameters.is_null() {
            return;
        }
        let info = &*callback_info;
        let request = (*callback_parameters).Anonymous.FetchData;
        let context = &*(info.CallbackContext as *const Arc<CallbackContext>);
        let identity = slice::from_raw_parts(
            info.FileIdentity.cast::<u8>(),
            info.FileIdentityLength as usize,
        );
        let (file_id, _) = match parse_identity(identity) {
            Some(value) => value,
            None => {
                transfer(
                    info,
                    request.RequiredFileOffset,
                    &[],
                    STATUS_CLOUD_FILE_NETWORK_UNAVAILABLE,
                );
                return;
            }
        };
        let result = context
            .runtime
            .block_on(context.client.download_verified(file_id));
        match result {
            Ok((_manifest, data)) => {
                let start = usize::try_from(request.RequiredFileOffset)
                    .unwrap_or(usize::MAX)
                    .min(data.len());
                let requested = if request.RequiredLength < 0 {
                    data.len() - start
                } else {
                    request.RequiredLength as usize
                };
                let end = start.saturating_add(requested).min(data.len());
                transfer(
                    info,
                    request.RequiredFileOffset,
                    &data[start..end],
                    STATUS_SUCCESS,
                );
            }
            Err(_) => transfer(
                info,
                request.RequiredFileOffset,
                &[],
                STATUS_CLOUD_FILE_NETWORK_UNAVAILABLE,
            ),
        }
    }
}

unsafe extern "system" fn file_closed(
    callback_info: *const CF_CALLBACK_INFO,
    callback_parameters: *const CF_CALLBACK_PARAMETERS,
) {
    unsafe {
        if callback_info.is_null() || callback_parameters.is_null() {
            return;
        }
        let info = &*callback_info;
        let closed = (*callback_parameters).Anonymous.CloseCompletion;
        if closed.Flags & CF_CALLBACK_CLOSE_COMPLETION_FLAG_DELETED != 0
            || info.NormalizedPath.is_null()
        {
            return;
        }
        let identity = slice::from_raw_parts(
            info.FileIdentity.cast::<u8>(),
            info.FileIdentityLength as usize,
        );
        let Some((file_id, base_version)) = parse_identity(identity) else {
            return;
        };
        let path = path_from_wide(info.NormalizedPath);
        let context = Arc::clone(&*(info.CallbackContext as *const Arc<CallbackContext>));
        let runtime = Arc::clone(&context.runtime);
        let file_id = file_id.to_owned();
        let base_version = base_version.to_owned();
        runtime.spawn(async move {
            if is_in_sync(&path).unwrap_or(true) {
                return;
            }
            let Ok(data) = read_stable(&path) else {
                return;
            };
            let Ok(relative) = path.strip_prefix(&context.root) else {
                return;
            };
            let relative = relative.to_string_lossy().replace('\\', "/");
            let hash = blake3::hash(&data).to_hex().to_string();
            let idempotency = format!("windows:{file_id}:{base_version}:{hash}");
            let Ok(result) = context
                .client
                .upload_bytes(
                    &context.space_id,
                    &relative,
                    Some(&file_id),
                    Some(&base_version),
                    &data,
                    &idempotency,
                )
                .await
            else {
                return;
            };
            if !result.conflict {
                let identity = encode_identity(&result.file_id, &result.version_id);
                let _ = mark_uploaded(&path, &identity);
            }
        });
    }
}

unsafe fn transfer(info: &CF_CALLBACK_INFO, offset: i64, data: &[u8], status: i32) {
    let operation = CF_OPERATION_INFO {
        StructSize: size_of::<CF_OPERATION_INFO>() as u32,
        Type: CF_OPERATION_TYPE_TRANSFER_DATA,
        ConnectionKey: info.ConnectionKey,
        TransferKey: info.TransferKey,
        CorrelationVector: info.CorrelationVector,
        SyncStatus: null(),
        RequestKey: info.RequestKey,
    };
    let mut parameters = CF_OPERATION_PARAMETERS {
        ParamSize: (offset_of!(CF_OPERATION_PARAMETERS, Anonymous)
            + size_of::<CF_OPERATION_PARAMETERS_0_0>()) as u32,
        Anonymous: CF_OPERATION_PARAMETERS_0 {
            TransferData: CF_OPERATION_PARAMETERS_0_0 {
                Flags: CF_OPERATION_TRANSFER_DATA_FLAG_NONE,
                CompletionStatus: status,
                Buffer: if data.is_empty() {
                    null()
                } else {
                    data.as_ptr().cast::<c_void>()
                },
                Offset: offset,
                Length: data.len() as i64,
            },
        },
    };
    unsafe {
        CfExecute(&operation, &mut parameters);
    }
}

struct ProtectedHandle(HANDLE);

impl ProtectedHandle {
    fn open(path: &Path, flags: CF_OPEN_FILE_FLAGS) -> anyhow::Result<Self> {
        let path = wide(path);
        let mut handle = null_mut();
        check_hresult(
            unsafe { CfOpenFileWithOplock(path.as_ptr(), flags, &mut handle) },
            "Cloud-Datei öffnen",
        )?;
        Ok(Self(handle))
    }
}

impl Drop for ProtectedHandle {
    fn drop(&mut self) {
        unsafe { CfCloseHandle(self.0) }
    }
}

fn placeholder_information(handle: HANDLE) -> anyhow::Result<CF_PLACEHOLDER_STANDARD_INFO> {
    let mut information = CF_PLACEHOLDER_STANDARD_INFO::default();
    let mut returned = 0;
    check_hresult(
        unsafe {
            CfGetPlaceholderInfo(
                handle,
                CF_PLACEHOLDER_INFO_STANDARD,
                (&mut information as *mut CF_PLACEHOLDER_STANDARD_INFO).cast(),
                size_of::<CF_PLACEHOLDER_STANDARD_INFO>() as u32,
                &mut returned,
            )
        },
        "Platzhalterstatus lesen",
    )?;
    Ok(information)
}

fn is_in_sync(path: &Path) -> anyhow::Result<bool> {
    let handle = ProtectedHandle::open(path, CF_OPEN_FILE_FLAG_NONE)?;
    Ok(placeholder_information(handle.0)?.InSyncState == CF_IN_SYNC_STATE_IN_SYNC)
}

fn mark_uploaded(path: &Path, identity: &[u8]) -> anyhow::Result<()> {
    let handle = ProtectedHandle::open(path, CF_OPEN_FILE_FLAG_WRITE_ACCESS)?;
    check_hresult(
        unsafe {
            CfUpdatePlaceholder(
                handle.0,
                null(),
                identity.as_ptr().cast(),
                identity.len() as u32,
                null(),
                0,
                CF_UPDATE_FLAG_MARK_IN_SYNC,
                null_mut(),
                null_mut(),
            )
        },
        "Hochgeladene Datei bestätigen",
    )
}

fn read_stable(path: &Path) -> anyhow::Result<Vec<u8>> {
    let before = std::fs::metadata(path)?;
    let data = std::fs::read(path)?;
    let after = std::fs::metadata(path)?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        bail!("Datei wurde während des Uploads geändert");
    }
    Ok(data)
}

fn encode_identity(file_id: &str, version_id: &str) -> Vec<u8> {
    format!("{file_id}\n{version_id}").into_bytes()
}

fn parse_identity(identity: &[u8]) -> Option<(&str, &str)> {
    std::str::from_utf8(identity).ok()?.split_once('\n')
}

unsafe fn path_from_wide(value: *const u16) -> PathBuf {
    let mut length = 0;
    unsafe {
        while *value.add(length) != 0 {
            length += 1;
        }
        PathBuf::from(OsString::from_wide(slice::from_raw_parts(value, length)))
    }
}

fn check_hresult(result: i32, operation: &str) -> anyhow::Result<()> {
    if result < 0 {
        bail!(
            "{operation} fehlgeschlagen: HRESULT 0x{:08X}",
            result as u32
        )
    }
    Ok(())
}

fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}
