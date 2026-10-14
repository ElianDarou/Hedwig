#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod model;
use model::*;
use notify::Watcher;
use serde::Serialize;
#[cfg(any(target_os = "linux", windows))]
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Arc;
use tauri::{
    Emitter, Manager,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
};
use tokio::sync::Mutex;
use tauri_plugin_autostart::ManagerExt as AutoStartExt;
use walkdir::WalkDir;

#[derive(Serialize)]
struct IgnoreMatch {
    path: String,
    rule: String,
    origin: String,
}

#[derive(Serialize)]
struct ConflictFile {
    folder_id: String,
    path: String,
}

struct RuntimeState {
    model: Arc<Mutex<AppModel>>,
    config_path: std::path::PathBuf,
    sync_gate: Mutex<()>,
    #[cfg(target_os = "linux")]
    mounts: std::sync::Mutex<HashMap<String, hedwig_mount::linux::MountHandle>>,
    #[cfg(windows)]
    windows_providers:
        std::sync::Mutex<HashMap<String, hedwig_files_windows::ProviderSession>>,
}

fn paths_overlap(left: &std::path::Path, right: &std::path::Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

fn credential_token(device_id: &str) -> Result<String, String> {
    let current = keyring::Entry::new("hedwig", device_id).map_err(|error| error.to_string())?;
    if let Ok(token) = current.get_password() {
        return Ok(token);
    }
    let legacy = keyring::Entry::new("cloud-sync", device_id).map_err(|error| error.to_string())?;
    let token = legacy.get_password().map_err(|error| error.to_string())?;
    current
        .set_password(&token)
        .map_err(|error| error.to_string())?;
    Ok(token)
}

fn delete_credentials(device_id: &str) {
    for service in ["hedwig", "cloud-sync"] {
        let _ = keyring::Entry::new(service, device_id)
            .and_then(|entry| entry.delete_credential());
    }
}

#[tauri::command]
async fn model(state: tauri::State<'_, RuntimeState>) -> Result<AppModel, String> {
    Ok(state.model.lock().await.clone())
}

#[tauri::command]
async fn login(
    state: tauri::State<'_, RuntimeState>,
    server: String,
    username: String,
    password: String,
    otp: Option<String>,
) -> Result<AppModel, String> {
    let (_, session) = hedwig_client::CloudClient::login_with_otp(
        &server,
        &username,
        &password,
        "Hedwig Desktop",
        otp.as_deref().filter(|value| !value.is_empty()),
    )
            .await
            .map_err(|e| e.to_string())?;
    keyring::Entry::new("hedwig", &session.device_id)
        .map_err(|e| e.to_string())?
        .set_password(&session.token)
        .map_err(|e| e.to_string())?;
    let mut model = state.model.lock().await;
    model.account = Some(Account {
        server,
        username: session.username,
        device_id: session.device_id,
        personal_space_id: session.personal_space_id,
    });
    save(&state.config_path, &model).map_err(|e| e.to_string())?;
    Ok(model.clone())
}

#[tauri::command]
async fn spaces(state: tauri::State<'_, RuntimeState>) -> Result<Vec<hedwig_client::Space>, String> {
    let model = state.model.lock().await.clone();
    let account = model.account.ok_or("Nicht angemeldet")?;
    let token = credential_token(&account.device_id)?;
    hedwig_client::CloudClient::new(&account.server, token)
        .map_err(|e| e.to_string())?
        .spaces()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn logout(state: tauri::State<'_, RuntimeState>) -> Result<AppModel, String> {
    let account = state.model.lock().await.account.clone();
    if let Some(account) = &account {
        let token = credential_token(&account.device_id)?;
        let client = hedwig_client::CloudClient::new(&account.server, token)
            .map_err(|e| e.to_string())?;
        if let Err(error) = client.logout().await
            && !error.is_unauthorized()
        {
            return Err(format!("Abmelden am Server fehlgeschlagen: {error}"));
        }
    }
    #[cfg(target_os = "linux")]
    {
        let handles: Vec<_> = state.mounts.lock().unwrap().drain().map(|(_, handle)| handle).collect();
        for handle in handles {
            tokio::task::spawn_blocking(move || handle.unmount())
                .await
                .map_err(|error| error.to_string())?
                .map_err(|error| error.to_string())?;
        }
    }
    #[cfg(windows)]
    state.windows_providers.lock().unwrap().clear();
    let mut model = state.model.lock().await;
    if let Some(account) = &account {
        delete_credentials(&account.device_id);
    }
    model.account = None;
    save(&state.config_path, &model).map_err(|e| e.to_string())?;
    Ok(model.clone())
}

#[tauri::command]
async fn add_folder(
    state: tauri::State<'_, RuntimeState>,
    path: String,
    space_id: String,
    mode: SyncMode,
) -> Result<AppModel, String> {
    let canonical =
        std::fs::canonicalize(&path).map_err(|e| format!("Ordner ist nicht erreichbar: {e}"))?;
    if !canonical.is_dir() {
        return Err("Der gewählte Pfad ist kein Ordner".into());
    }
    let mut model = state.model.lock().await;
    if model
        .folders
        .iter()
        .any(|folder| paths_overlap(&folder.path, &canonical))
    {
        return Err("Dieser Ordner wird bereits synchronisiert".into());
    }
    if model
        .linux_mounts
        .iter()
        .any(|mount| paths_overlap(&mount.path, &canonical))
        || model
            .windows_roots
            .iter()
            .any(|root| paths_overlap(&root.path, &canonical))
    {
        return Err("Ein Pfad darf nicht zugleich virtueller Zugriff und klassischer Sync-Ordner sein".into());
    }
    model.folders.push(Folder {
        id: uuid::Uuid::new_v4().to_string(),
        path: canonical,
        space_id,
        mode,
        paused: false,
        last_report: None,
        last_success: None,
        last_error: None,
        selective_excluded: Vec::new(),
    });
    save(&state.config_path, &model).map_err(|e| e.to_string())?;
    Ok(model.clone())
}

#[tauri::command]
async fn set_paused(
    state: tauri::State<'_, RuntimeState>,
    folder_id: String,
    paused: bool,
) -> Result<AppModel, String> {
    let mut model = state.model.lock().await;
    let folder = model
        .folders
        .iter_mut()
        .find(|f| f.id == folder_id)
        .ok_or("Ordner nicht gefunden")?;
    folder.paused = paused;
    save(&state.config_path, &model).map_err(|e| e.to_string())?;
    Ok(model.clone())
}

#[tauri::command]
async fn set_mode(
    state: tauri::State<'_, RuntimeState>,
    folder_id: String,
    mode: SyncMode,
) -> Result<AppModel, String> {
    let mut model = state.model.lock().await;
    let folder = model
        .folders
        .iter_mut()
        .find(|folder| folder.id == folder_id)
        .ok_or("Ordner nicht gefunden")?;
    folder.mode = mode;
    save(&state.config_path, &model).map_err(|error| error.to_string())?;
    Ok(model.clone())
}

#[tauri::command]
async fn set_selective_sync(
    state: tauri::State<'_, RuntimeState>,
    folder_id: String,
    paths: Vec<String>,
) -> Result<AppModel, String> {
    let mut normalized = Vec::new();
    for path in paths {
        let trimmed = path.trim().trim_matches('/');
        if trimmed.is_empty() {
            continue;
        }
        let path = hedwig_core::CloudPath::parse(trimmed)
            .map_err(|error| format!("Ungültiger Cloud-Ordner {trimmed}: {error}"))?
            .as_str()
            .to_owned();
        normalized.push(path);
    }
    normalized.sort();
    normalized.dedup();
    for (index, path) in normalized.iter().enumerate() {
        if normalized
            .iter()
            .enumerate()
            .any(|(other_index, other)| {
                index != other_index && path.starts_with(&format!("{other}/"))
            })
        {
            return Err("Unterordner eines bereits ausgeschlossenen Ordners bitte entfernen".into());
        }
    }
    let (paused, added) = {
        let model = state.model.lock().await;
        let folder = model
            .folders
            .iter()
            .find(|folder| folder.id == folder_id)
            .ok_or("Ordner nicht gefunden")?;
        (
            folder.paused,
            normalized
                .iter()
                .any(|path| !folder.selective_excluded.contains(path)),
        )
    };
    if paused && added {
        return Err("Vor einem neuen selektiven Ausschluss muss der Ordner fortgesetzt und einmal erfolgreich abgeglichen werden".into());
    }
    if added {
        run_sync(&state, Some(&folder_id), false).await?;
        let model = state.model.lock().await;
        let folder = model
            .folders
            .iter()
            .find(|folder| folder.id == folder_id)
            .ok_or("Ordner nicht gefunden")?;
        if let Some(error) = &folder.last_error {
            return Err(format!(
                "Selektiver Sync wurde nicht geändert, weil der Sicherheitsabgleich fehlgeschlagen ist: {error}"
            ));
        }
    }
    {
        let mut model = state.model.lock().await;
        let folder = model
            .folders
            .iter_mut()
            .find(|folder| folder.id == folder_id)
            .ok_or("Ordner nicht gefunden")?;
        folder.selective_excluded = normalized;
        save(&state.config_path, &model).map_err(|error| error.to_string())?;
    }
    run_sync(&state, Some(&folder_id), false).await?;
    Ok(state.model.lock().await.clone())
}

#[tauri::command]
async fn set_safety_limits(
    state: tauri::State<'_, RuntimeState>,
    delete_limit: usize,
    delete_percent: usize,
) -> Result<AppModel, String> {
    if delete_limit == 0 || !(1..=100).contains(&delete_percent) {
        return Err("Grenzen müssen positiv sein; der Prozentwert darf höchstens 100 sein".into());
    }
    let mut model = state.model.lock().await;
    model.settings.delete_limit = delete_limit;
    model.settings.delete_percent = delete_percent;
    save(&state.config_path, &model).map_err(|error| error.to_string())?;
    Ok(model.clone())
}

#[tauri::command]
async fn set_bandwidth_limits(
    state: tauri::State<'_, RuntimeState>,
    upload_limit_kib: u64,
    download_limit_kib: u64,
    parallel_transfers: usize,
) -> Result<AppModel, String> {
    if !(1..=64).contains(&parallel_transfers) {
        return Err("Gleichzeitige Übertragungen müssen zwischen 1 und 64 liegen".into());
    }
    let mut model = state.model.lock().await;
    model.settings.upload_limit_kib = upload_limit_kib;
    model.settings.download_limit_kib = download_limit_kib;
    model.settings.parallel_transfers = parallel_transfers;
    save(&state.config_path, &model).map_err(|error| error.to_string())?;
    Ok(model.clone())
}

#[tauri::command]
fn autostart_status(app: tauri::AppHandle) -> Result<bool, String> {
    app.autolaunch().is_enabled().map_err(|error| error.to_string())
}

#[tauri::command]
fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<bool, String> {
    if enabled {
        app.autolaunch().enable()
    } else {
        app.autolaunch().disable()
    }
    .map_err(|error| error.to_string())?;
    app.autolaunch().is_enabled().map_err(|error| error.to_string())
}

#[tauri::command]
async fn local_ignore(
    state: tauri::State<'_, RuntimeState>,
    folder_id: String,
) -> Result<String, String> {
    let model = state.model.lock().await;
    model
        .folders
        .iter()
        .find(|folder| folder.id == folder_id)
        .ok_or("Ordner nicht gefunden")?;
    let directory = folder_state_dir(&state.config_path, &folder_id);
    match std::fs::read_to_string(directory.join("local.hedwigignore"))
        .or_else(|_| std::fs::read_to_string(directory.join("local.cloudignore")))
    {
        Ok(contents) => Ok(contents),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
        Err(error) => Err(error.to_string()),
    }
}

#[tauri::command]
async fn preview_local_ignore(
    state: tauri::State<'_, RuntimeState>,
    folder_id: String,
    contents: String,
) -> Result<Vec<IgnoreMatch>, String> {
    let model = state.model.lock().await;
    let folder = model
        .folders
        .iter()
        .find(|folder| folder.id == folder_id)
        .ok_or("Ordner nicht gefunden")?;
    let legacy = std::fs::read_to_string(folder.path.join(".cloudignore")).unwrap_or_default();
    let shared = format!(
        "{legacy}\n{}",
        std::fs::read_to_string(folder.path.join(".hedwigignore")).unwrap_or_default()
    );
    let rules = hedwig_core::IgnoreRules::parse(&shared, &contents).map_err(|error| error.to_string())?;
    let mut matches = Vec::new();
    for item in WalkDir::new(&folder.path).follow_links(false).into_iter().filter_map(Result::ok) {
        if item.path() == folder.path || item.file_type().is_symlink() {
            continue;
        }
        let relative = item
            .path()
            .strip_prefix(&folder.path)
            .map_err(|error| error.to_string())?
            .to_string_lossy()
            .replace('\\', "/");
        let decision = rules.evaluate(std::path::Path::new(&relative), item.file_type().is_dir());
        if decision.ignored {
            matches.push(IgnoreMatch {
                path: relative,
                rule: decision.rule.unwrap_or_default(),
                origin: match decision.origin {
                    Some(hedwig_core::RuleOrigin::Local) => "Lokal".into(),
                    Some(hedwig_core::RuleOrigin::Shared) => "Gemeinsam".into(),
                    None => "Unbekannt".into(),
                },
            });
            if matches.len() == 200 {
                break;
            }
        }
    }
    Ok(matches)
}

#[tauri::command]
async fn save_local_ignore(
    state: tauri::State<'_, RuntimeState>,
    folder_id: String,
    contents: String,
) -> Result<(), String> {
    hedwig_core::IgnoreRules::parse("", &contents).map_err(|error| error.to_string())?;
    let model = state.model.lock().await;
    model
        .folders
        .iter()
        .find(|folder| folder.id == folder_id)
        .ok_or("Ordner nicht gefunden")?;
    let directory = folder_state_dir(&state.config_path, &folder_id);
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let path = directory.join("local.hedwigignore");
    let temporary = directory.join("local.hedwigignore.tmp");
    std::fs::write(&temporary, contents).map_err(|error| error.to_string())?;
    std::fs::File::open(&temporary)
        .and_then(|file| file.sync_all())
        .map_err(|error| error.to_string())?;
    std::fs::rename(temporary, path).map_err(|error| error.to_string())
}

#[tauri::command]
async fn conflicts(state: tauri::State<'_, RuntimeState>) -> Result<Vec<ConflictFile>, String> {
    let model = state.model.lock().await;
    let mut conflicts = Vec::new();
    for folder in &model.folders {
        for item in WalkDir::new(&folder.path)
            .follow_links(false)
            .into_iter()
            .filter_map(Result::ok)
        {
            if item.file_type().is_file()
                && item.file_name().to_string_lossy().contains("(Lokaler Konflikt ")
            {
                conflicts.push(ConflictFile {
                    folder_id: folder.id.clone(),
                    path: item.path().to_string_lossy().into_owned(),
                });
            }
        }
    }
    Ok(conflicts)
}

#[tauri::command]
fn linux_mount_supported() -> bool {
    cfg!(target_os = "linux")
}

#[cfg(target_os = "linux")]
async fn mount_profile(
    state: &RuntimeState,
    profile: &LinuxMount,
    account: &Account,
) -> Result<(), String> {
    if state.mounts.lock().unwrap().contains_key(&profile.id) {
        return Ok(());
    }
    let token = credential_token(&account.device_id)?;
    let config = hedwig_mount::linux::MountConfig {
        server: account.server.clone(),
        token,
        space: profile.space_id.clone(),
        mountpoint: profile.path.clone(),
        cache_dir: state
            .config_path
            .parent()
            .unwrap()
            .join("mount-cache")
            .join(&profile.id),
        refresh_seconds: 15,
    };
    let handle = tokio::task::spawn_blocking(move || hedwig_mount::linux::start_mount(config))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    state
        .mounts
        .lock()
        .unwrap()
        .insert(profile.id.clone(), handle);
    Ok(())
}

#[cfg(target_os = "linux")]
#[tauri::command]
async fn add_linux_mount(
    state: tauri::State<'_, RuntimeState>,
    path: String,
    space_id: String,
) -> Result<AppModel, String> {
    let canonical = std::fs::canonicalize(&path)
        .map_err(|error| format!("Mountpunkt ist nicht erreichbar: {error}"))?;
    if !canonical.is_dir() {
        return Err("Der Mountpunkt ist kein Ordner".into());
    }
    if std::fs::read_dir(&canonical)
        .map_err(|error| error.to_string())?
        .next()
        .is_some()
    {
        return Err("Der Mountpunkt muss leer sein".into());
    }
    let (account, profile) = {
        let model = state.model.lock().await;
        if model
            .folders
            .iter()
            .any(|folder| paths_overlap(&folder.path, &canonical))
        {
            return Err("Ein Pfad darf nicht zugleich virtueller Mount und Sync-Ordner sein".into());
        }
        if model
            .linux_mounts
            .iter()
            .any(|mount| paths_overlap(&mount.path, &canonical))
            || model
                .windows_roots
                .iter()
                .any(|root| paths_overlap(&root.path, &canonical))
        {
            return Err("Dieser Mountpunkt ist bereits eingerichtet".into());
        }
        let account = model.account.clone().ok_or("Nicht angemeldet")?;
        let profile = LinuxMount {
            id: uuid::Uuid::new_v4().to_string(),
            path: canonical,
            space_id,
        };
        (account, profile)
    };
    mount_profile(&state, &profile, &account).await?;
    let mut model = state.model.lock().await;
    model.linux_mounts.push(profile);
    save(&state.config_path, &model).map_err(|error| error.to_string())?;
    Ok(model.clone())
}

#[cfg(not(target_os = "linux"))]
#[tauri::command]
async fn add_linux_mount(
    _state: tauri::State<'_, RuntimeState>,
    _path: String,
    _space_id: String,
) -> Result<AppModel, String> {
    Err("Virtuelle Cloud-Mounts werden nur unter Linux unterstützt".into())
}

#[cfg(target_os = "linux")]
#[tauri::command]
async fn start_linux_mount(
    state: tauri::State<'_, RuntimeState>,
    mount_id: String,
) -> Result<Vec<String>, String> {
    let (account, profile) = {
        let model = state.model.lock().await;
        (
            model.account.clone().ok_or("Nicht angemeldet")?,
            model
                .linux_mounts
                .iter()
                .find(|profile| profile.id == mount_id)
                .cloned()
                .ok_or("Mount nicht gefunden")?,
        )
    };
    mount_profile(&state, &profile, &account).await?;
    Ok(state.mounts.lock().unwrap().keys().cloned().collect())
}

#[cfg(not(target_os = "linux"))]
#[tauri::command]
async fn start_linux_mount(
    _state: tauri::State<'_, RuntimeState>,
    _mount_id: String,
) -> Result<Vec<String>, String> {
    Err("Virtuelle Cloud-Mounts werden nur unter Linux unterstützt".into())
}

#[cfg(target_os = "linux")]
#[tauri::command]
async fn stop_linux_mount(
    state: tauri::State<'_, RuntimeState>,
    mount_id: String,
) -> Result<Vec<String>, String> {
    let handle = { state.mounts.lock().unwrap().remove(&mount_id) };
    if let Some(handle) = handle {
        tokio::task::spawn_blocking(move || handle.unmount())
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
    }
    Ok(state.mounts.lock().unwrap().keys().cloned().collect())
}

#[cfg(not(target_os = "linux"))]
#[tauri::command]
async fn stop_linux_mount(
    _state: tauri::State<'_, RuntimeState>,
    _mount_id: String,
) -> Result<Vec<String>, String> {
    Ok(Vec::new())
}

#[tauri::command]
async fn linux_mount_status(state: tauri::State<'_, RuntimeState>) -> Result<Vec<String>, String> {
    #[cfg(target_os = "linux")]
    {
        return Ok(state.mounts.lock().unwrap().keys().cloned().collect());
    }
    #[cfg(not(target_os = "linux"))]
    Ok(Vec::new())
}

#[tauri::command]
fn windows_on_demand_supported() -> bool {
    cfg!(windows)
}

#[cfg(windows)]
async fn start_windows_profile(
    state: &RuntimeState,
    profile: &WindowsRoot,
    account: &Account,
) -> Result<(), String> {
    if state
        .windows_providers
        .lock()
        .unwrap()
        .contains_key(&profile.id)
    {
        return Ok(());
    }
    let token = credential_token(&account.device_id)?;
    let config = hedwig_files_windows::ProviderConfig {
        server: account.server.clone(),
        token,
        space_id: profile.space_id.clone(),
        root: profile.path.clone(),
    };
    let provider = tokio::task::spawn_blocking(move || {
        hedwig_files_windows::ProviderSession::start(config)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(|error| error.to_string())?;
    state
        .windows_providers
        .lock()
        .unwrap()
        .insert(profile.id.clone(), provider);
    Ok(())
}

#[cfg(windows)]
#[tauri::command]
async fn add_windows_root(
    state: tauri::State<'_, RuntimeState>,
    path: String,
    space_id: String,
) -> Result<AppModel, String> {
    let canonical = std::fs::canonicalize(&path)
        .map_err(|error| format!("Files-on-Demand-Ordner ist nicht erreichbar: {error}"))?;
    if !canonical.is_dir() {
        return Err("Der gewählte Pfad ist kein Ordner".into());
    }
    if std::fs::read_dir(&canonical)
        .map_err(|error| error.to_string())?
        .next()
        .is_some()
    {
        return Err("Der Files-on-Demand-Ordner muss beim Einrichten leer sein".into());
    }
    let (account, profile) = {
        let model = state.model.lock().await;
        if model
            .folders
            .iter()
            .any(|folder| paths_overlap(&folder.path, &canonical))
        {
            return Err("Ein Pfad darf nicht zugleich Files-on-Demand und klassischer Sync-Ordner sein".into());
        }
        if model
            .windows_roots
            .iter()
            .any(|root| paths_overlap(&root.path, &canonical))
            || model
                .linux_mounts
                .iter()
                .any(|mount| paths_overlap(&mount.path, &canonical))
        {
            return Err("Dieser Files-on-Demand-Ordner ist bereits eingerichtet".into());
        }
        (
            model.account.clone().ok_or("Nicht angemeldet")?,
            WindowsRoot {
                id: uuid::Uuid::new_v4().to_string(),
                path: canonical,
                space_id,
            },
        )
    };
    start_windows_profile(&state, &profile, &account).await?;
    let mut model = state.model.lock().await;
    model.windows_roots.push(profile);
    save(&state.config_path, &model).map_err(|error| error.to_string())?;
    Ok(model.clone())
}

#[cfg(not(windows))]
#[tauri::command]
async fn add_windows_root(
    _state: tauri::State<'_, RuntimeState>,
    _path: String,
    _space_id: String,
) -> Result<AppModel, String> {
    Err("Files-on-Demand ist nur unter Windows verfügbar".into())
}

#[cfg(windows)]
#[tauri::command]
async fn start_windows_root(
    state: tauri::State<'_, RuntimeState>,
    root_id: String,
) -> Result<Vec<String>, String> {
    let (account, profile) = {
        let model = state.model.lock().await;
        (
            model.account.clone().ok_or("Nicht angemeldet")?,
            model.windows_roots.iter().find(|root| root.id == root_id).cloned().ok_or("Files-on-Demand-Ordner nicht gefunden")?,
        )
    };
    start_windows_profile(&state, &profile, &account).await?;
    Ok(state.windows_providers.lock().unwrap().keys().cloned().collect())
}

#[cfg(not(windows))]
#[tauri::command]
async fn start_windows_root(
    _state: tauri::State<'_, RuntimeState>,
    _root_id: String,
) -> Result<Vec<String>, String> {
    Err("Files-on-Demand ist nur unter Windows verfügbar".into())
}

#[tauri::command]
async fn stop_windows_root(
    state: tauri::State<'_, RuntimeState>,
    root_id: String,
) -> Result<Vec<String>, String> {
    #[cfg(windows)]
    {
        state.windows_providers.lock().unwrap().remove(&root_id);
        return Ok(state.windows_providers.lock().unwrap().keys().cloned().collect());
    }
    #[cfg(not(windows))]
    {
        let _ = (state, root_id);
        Ok(Vec::new())
    }
}

#[tauri::command]
async fn windows_root_status(
    state: tauri::State<'_, RuntimeState>,
) -> Result<Vec<String>, String> {
    #[cfg(windows)]
    return Ok(state.windows_providers.lock().unwrap().keys().cloned().collect());
    #[cfg(not(windows))]
    {
        let _ = state;
        Ok(Vec::new())
    }
}

#[tauri::command]
async fn windows_keep_offline(
    state: tauri::State<'_, RuntimeState>,
    root_id: String,
    path: String,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        let model = state.model.lock().await;
        let root = model.windows_roots.iter().find(|root| root.id == root_id).ok_or("Files-on-Demand-Ordner nicht gefunden")?;
        let target = std::path::PathBuf::from(path);
        if !target.starts_with(&root.path) { return Err("Pfad liegt nicht im Files-on-Demand-Ordner".into()); }
        return tokio::task::spawn_blocking(move || hedwig_files_windows::ProviderSession::keep_offline(&target)).await.map_err(|error| error.to_string())?.map_err(|error| error.to_string());
    }
    #[cfg(not(windows))]
    {
        let _ = (state, root_id, path);
        Err("Files-on-Demand ist nur unter Windows verfügbar".into())
    }
}

#[tauri::command]
async fn windows_free_space(
    state: tauri::State<'_, RuntimeState>,
    root_id: String,
    path: String,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        let model = state.model.lock().await;
        let root = model.windows_roots.iter().find(|root| root.id == root_id).ok_or("Files-on-Demand-Ordner nicht gefunden")?;
        let target = std::path::PathBuf::from(path);
        if !target.starts_with(&root.path) { return Err("Pfad liegt nicht im Files-on-Demand-Ordner".into()); }
        return tokio::task::spawn_blocking(move || hedwig_files_windows::ProviderSession::free_space(&target)).await.map_err(|error| error.to_string())?.map_err(|error| error.to_string());
    }
    #[cfg(not(windows))]
    {
        let _ = (state, root_id, path);
        Err("Files-on-Demand ist nur unter Windows verfügbar".into())
    }
}

#[tauri::command]
async fn remove_windows_root(
    state: tauri::State<'_, RuntimeState>,
    root_id: String,
) -> Result<AppModel, String> {
    #[cfg(windows)]
    {
        let root_path = {
            let model = state.model.lock().await;
            model
                .windows_roots
                .iter()
                .find(|root| root.id == root_id)
                .map(|root| root.path.clone())
                .ok_or("Files-on-Demand-Ordner nicht gefunden")?
        };
        let provider = { state.windows_providers.lock().unwrap().remove(&root_id) };
        tokio::task::spawn_blocking(move || match provider {
            Some(provider) => provider.unregister(),
            None => hedwig_files_windows::ProviderSession::unregister_path(&root_path),
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())?;
    }
    let mut model = state.model.lock().await;
    model.windows_roots.retain(|root| root.id != root_id);
    save(&state.config_path, &model).map_err(|error| error.to_string())?;
    Ok(model.clone())
}

#[tauri::command]
async fn remove_linux_mount(
    state: tauri::State<'_, RuntimeState>,
    mount_id: String,
) -> Result<AppModel, String> {
    #[cfg(target_os = "linux")]
    if let Some(handle) = { state.mounts.lock().unwrap().remove(&mount_id) } {
        tokio::task::spawn_blocking(move || handle.unmount())
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())?;
    }
    let mut model = state.model.lock().await;
    model.linux_mounts.retain(|profile| profile.id != mount_id);
    save(&state.config_path, &model).map_err(|error| error.to_string())?;
    Ok(model.clone())
}

#[tauri::command]
async fn remove_folder(
    state: tauri::State<'_, RuntimeState>,
    folder_id: String,
) -> Result<AppModel, String> {
    let mut model = state.model.lock().await;
    model.folders.retain(|folder| folder.id != folder_id);
    save(&state.config_path, &model).map_err(|e| e.to_string())?;
    Ok(model.clone())
}

#[tauri::command]
async fn sync_now(
    state: tauri::State<'_, RuntimeState>,
    folder_id: String,
) -> Result<AppModel, String> {
    run_sync(&state, Some(&folder_id), false).await?;
    Ok(state.model.lock().await.clone())
}

#[tauri::command]
async fn confirm_deletions(
    state: tauri::State<'_, RuntimeState>,
    folder_id: String,
) -> Result<AppModel, String> {
    run_sync(&state, Some(&folder_id), true).await?;
    Ok(state.model.lock().await.clone())
}

async fn run_sync(
    state: &RuntimeState,
    only: Option<&str>,
    allow_deletions_once: bool,
) -> Result<(), String> {
    let _gate = state.sync_gate.lock().await;
    let snapshot = state.model.lock().await.clone();
    let Some(account) = snapshot.account else {
        return Ok(());
    };
    let token = credential_token(&account.device_id)?;
    for folder in snapshot
        .folders
        .into_iter()
        .filter(|f| !f.paused && only.is_none_or(|id| id == f.id))
    {
        let client = hedwig_client::CloudClient::new(&account.server, token.clone())
            .map_err(|e| e.to_string())?
            .with_transfer_limits(
                snapshot.settings.upload_limit_kib.saturating_mul(1024),
                snapshot.settings.download_limit_kib.saturating_mul(1024),
            );
        let state_dir = folder_state_dir(&state.config_path, &folder.id);
        let result = hedwig_agent::sync::synchronize(
            client,
            hedwig_agent::sync::SyncConfig {
                space_id: folder.space_id.clone(),
                root: folder.path.clone(),
                state_dir,
                mode: match folder.mode {
                    SyncMode::TwoWay => hedwig_agent::sync::SyncMode::TwoWay,
                    SyncMode::UploadOnly => hedwig_agent::sync::SyncMode::UploadOnly,
                    SyncMode::DownloadOnly => hedwig_agent::sync::SyncMode::DownloadOnly,
                },
                delete_limit: snapshot.settings.delete_limit,
                delete_percent: snapshot.settings.delete_percent,
                allow_deletions_once,
                selective_excluded: folder.selective_excluded.clone(),
                parallel_transfers: snapshot.settings.parallel_transfers,
            },
        )
        .await;
        let mut model = state.model.lock().await;
        if let Some(target) = model.folders.iter_mut().find(|f| f.id == folder.id) {
            match result {
                Ok(report) => {
                    target.last_success = Some(chrono::Utc::now().to_rfc3339());
                    target.last_error = None;
                    target.last_report = Some(report)
                }
                Err(error) => target.last_error = Some(error.to_string()),
            }
        }
        save(&state.config_path, &model).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            let legacy_config_dir = app.path().config_dir()?.join("de.cloudsync.desktop");
            if !config_dir.exists() && legacy_config_dir.exists() {
                std::fs::rename(&legacy_config_dir, &config_dir)?;
            }
            std::fs::create_dir_all(&config_dir)?;
            let config_path = config_dir.join("desktop.json");
            let model = load(&config_path).unwrap_or_default();
            app.manage(RuntimeState {
                model: Arc::new(Mutex::new(model)),
                config_path,
                sync_gate: Mutex::new(()),
                #[cfg(target_os = "linux")]
                mounts: std::sync::Mutex::new(HashMap::new()),
                #[cfg(windows)]
                windows_providers: std::sync::Mutex::new(HashMap::new()),
            });
            let show = MenuItem::with_id(app, "show", "Hedwig öffnen", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Beenden", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let mut tray=TrayIconBuilder::new().tooltip("Hedwig").menu(&menu);
            if let Some(icon)=app.default_window_icon(){tray=tray.icon(icon.clone());}
            tray
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;
            if let Some(window)=app.get_webview_window("main"){
                let window_for_event=window.clone();
                window.on_window_event(move |event|if let tauri::WindowEvent::CloseRequested{api,..}=event{api.prevent_close();let _=window_for_event.hide();});
            }
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let state = handle.state::<RuntimeState>();
                let initial = state.model.lock().await.clone();
                if let Some(account) = initial.account {
                    #[cfg(target_os = "linux")]
                    for profile in initial.linux_mounts {
                        if let Err(error) = mount_profile(&state, &profile, &account).await {
                            eprintln!("Virtueller Mount {} konnte nicht gestartet werden: {error}", profile.path.display());
                        }
                    }
                    #[cfg(windows)]
                    for profile in initial.windows_roots {
                        if let Err(error) = start_windows_profile(&state, &profile, &account).await {
                            eprintln!("Files-on-Demand-Stamm {} konnte nicht gestartet werden: {error}", profile.path.display());
                        }
                    }
                }
                let (filesystem_tx, mut filesystem_rx) =
                    tokio::sync::mpsc::unbounded_channel();
                let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                    if event.is_ok() {
                        let _ = filesystem_tx.send(());
                    }
                })
                .map_err(|error| {
                    eprintln!("Dateisystemüberwachung ist nicht verfügbar; der vollständige Abgleich bleibt aktiv: {error}");
                    error
                })
                .ok();
                let mut watched = HashSet::new();
                loop {
                    let snapshot = state.model.lock().await.clone();
                    let desired: HashSet<_> = snapshot
                        .folders
                        .iter()
                        .filter(|folder| !folder.paused && folder.path.is_dir())
                        .map(|folder| folder.path.clone())
                        .collect();
                    if let Some(active_watcher) = watcher.as_mut() {
                        for removed in watched.difference(&desired).cloned().collect::<Vec<_>>() {
                            let _ = active_watcher.unwatch(&removed);
                            watched.remove(&removed);
                        }
                        for added in desired.difference(&watched).cloned().collect::<Vec<_>>() {
                            match active_watcher.watch(&added, notify::RecursiveMode::Recursive) {
                                Ok(()) => {
                                    watched.insert(added);
                                }
                                Err(error) => eprintln!(
                                    "Ordner {} kann nicht direkt überwacht werden; der vollständige Abgleich bleibt aktiv: {error}",
                                    added.display()
                                ),
                            }
                        }
                    }
                    let remote_client = snapshot.account.and_then(|account| {
                        credential_token(&account.device_id)
                            .ok()
                            .and_then(|token| hedwig_client::CloudClient::new(&account.server, token).ok())
                    });
                    let remote_wakeup = async move {
                        match remote_client {
                            Some(client) => {
                                if client
                                    .wait_for_change(std::time::Duration::from_secs(25))
                                    .await
                                    .is_err()
                                {
                                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                                }
                            }
                            None => tokio::time::sleep(std::time::Duration::from_secs(30)).await,
                        }
                    };
                    tokio::select! {
                        _ = remote_wakeup => {}
                        event = filesystem_rx.recv() => {
                            if event.is_some() {
                                tokio::time::sleep(std::time::Duration::from_millis(750)).await;
                                while filesystem_rx.try_recv().is_ok() {}
                            }
                        }
                    }
                    let _ = run_sync(&state, None, false).await;
                    let current = state.model.lock().await.clone();
                    let _ = handle.emit("model-changed", current);
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            model,
            login,
            logout,
            spaces,
            add_folder,
            remove_folder,
            set_paused,
            set_mode,
            set_selective_sync,
            set_safety_limits,
            set_bandwidth_limits,
            autostart_status,
            set_autostart,
            local_ignore,
            preview_local_ignore,
            save_local_ignore,
            conflicts,
            linux_mount_supported,
            add_linux_mount,
            start_linux_mount,
            stop_linux_mount,
            linux_mount_status,
            remove_linux_mount,
            windows_on_demand_supported,
            add_windows_root,
            start_windows_root,
            stop_windows_root,
            windows_root_status,
            windows_keep_offline,
            windows_free_space,
            remove_windows_root,
            sync_now,
            confirm_deletions
        ])
        .run(tauri::generate_context!())
        .expect("Hedwig desktop failed");
}
