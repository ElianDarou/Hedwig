use clap::{Parser, Subcommand};
use hedwig_server::{
    Database, create_backup, export_directory_tree, restore_backup, run_maintenance, verify_backup,
};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(version, about = "Offline administration for Hedwig")]
struct Arguments {
    #[arg(long, default_value = "./data/metadata.db")]
    database: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Reports whether the initial administrator still needs to be created.
    SetupStatus,
    /// Creates an atomic, hash-manifested snapshot in a new directory.
    Backup {
        #[arg(long, default_value = "./data")]
        data_dir: PathBuf,
        #[arg(long)]
        destination: PathBuf,
    },
    /// Checks every file hash and the SQLite integrity of a backup.
    VerifyBackup {
        #[arg(long)]
        source: PathBuf,
    },
    /// Restores a verified backup into a new or empty data directory.
    Restore {
        #[arg(long)]
        source: PathBuf,
        #[arg(long)]
        destination: PathBuf,
    },
    /// Applies version retention, expires trash, and removes unreferenced blocks.
    Maintenance {
        #[arg(long, default_value = "./data")]
        data_dir: PathBuf,
    },
    /// Exports current files and optionally pinned history as ordinary files.
    Export {
        #[arg(long, default_value = "./data")]
        data_dir: PathBuf,
        #[arg(long)]
        destination: PathBuf,
        #[arg(long)]
        include_pinned: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let arguments = Arguments::parse();
    match arguments.command {
        Command::SetupStatus => {
            let database = Database::open(arguments.database)?;
            println!("setup_required={}", database.setup_required()?);
        }
        Command::Backup {
            data_dir,
            destination,
        } => {
            let manifest = create_backup(&data_dir, &destination)?;
            println!(
                "backup={} files={} created_at={}",
                destination.display(),
                manifest.entries.len(),
                manifest.created_at
            );
        }
        Command::VerifyBackup { source } => {
            let manifest = verify_backup(&source)?;
            println!(
                "verified={} files={} created_at={}",
                source.display(),
                manifest.entries.len(),
                manifest.created_at
            );
        }
        Command::Restore {
            source,
            destination,
        } => {
            let manifest = restore_backup(&source, &destination)?;
            println!(
                "restored={} files={} backup_created_at={}",
                destination.display(),
                manifest.entries.len(),
                manifest.created_at
            );
        }
        Command::Maintenance { data_dir } => {
            let report = run_maintenance(&data_dir).await?;
            println!(
                "deleted_versions={} purged_files={} removed_blocks={}",
                report.deleted_versions, report.purged_files, report.removed_blocks
            );
        }
        Command::Export {
            data_dir,
            destination,
            include_pinned,
        } => {
            let report = export_directory_tree(&data_dir, &destination, include_pinned).await?;
            println!(
                "current_files={} historical_files={} bytes_written={}",
                report.current_files, report.historical_files, report.bytes_written
            );
        }
    }
    Ok(())
}
