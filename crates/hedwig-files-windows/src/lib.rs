#[cfg(windows)]
mod windows;

#[cfg(windows)]
pub use windows::{ProviderConfig, ProviderSession};

#[cfg(not(windows))]
pub struct ProviderSession;

#[cfg(not(windows))]
impl ProviderSession {
    pub fn unsupported() -> anyhow::Result<Self> {
        anyhow::bail!("Windows Files-on-Demand erfordert Windows 10 Version 1709 oder neuer")
    }
}
