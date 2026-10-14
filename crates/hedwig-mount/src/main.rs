#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    hedwig_mount::linux::run()
}

#[cfg(not(target_os = "linux"))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("Der virtuelle Cloud-Mount wird nur unter Linux unterstützt")
}
