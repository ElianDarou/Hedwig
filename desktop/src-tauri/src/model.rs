use hedwig_agent::sync::SyncReport;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppModel {
    pub account: Option<Account>,
    pub folders: Vec<Folder>,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub linux_mounts: Vec<LinuxMount>,
    #[serde(default)]
    pub windows_roots: Vec<WindowsRoot>,
}
impl Default for AppModel {
    fn default() -> Self {
        Self {
            account: None,
            folders: Vec::new(),
            settings: Settings::default(),
            linux_mounts: Vec::new(),
            windows_roots: Vec::new(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinuxMount {
    pub id: String,
    pub path: PathBuf,
    pub space_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowsRoot {
    pub id: String,
    pub path: PathBuf,
    pub space_id: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub delete_limit: usize,
    pub delete_percent: usize,
    #[serde(default)]
    pub upload_limit_kib: u64,
    #[serde(default)]
    pub download_limit_kib: u64,
    #[serde(default = "default_parallel_transfers")]
    pub parallel_transfers: usize,
}
fn default_parallel_transfers() -> usize { 16 }
impl Default for Settings {
    fn default() -> Self {
        Self {
            delete_limit: 25,
            delete_percent: 20,
            upload_limit_kib: 0,
            download_limit_kib: 0,
            parallel_transfers: default_parallel_transfers(),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub server: String,
    pub username: String,
    pub device_id: String,
    pub personal_space_id: String,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    TwoWay,
    UploadOnly,
    DownloadOnly,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Folder {
    pub id: String,
    pub path: PathBuf,
    pub space_id: String,
    pub mode: SyncMode,
    pub paused: bool,
    pub last_report: Option<SyncReport>,
    pub last_success: Option<String>,
    pub last_error: Option<String>,
    #[serde(default)]
    pub selective_excluded: Vec<String>,
}
pub fn load(path: &Path) -> anyhow::Result<AppModel> {
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}
pub fn save(path: &Path, model: &AppModel) -> anyhow::Result<()> {
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, serde_json::to_vec_pretty(model)?)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}
pub fn folder_state_dir(config_path: &Path, id: &str) -> PathBuf {
    config_path.parent().unwrap().join("folders").join(id)
}
