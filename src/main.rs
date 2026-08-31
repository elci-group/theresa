use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(
    name = "theresa",
    version,
    about = "Keep READMEs synchronized with changing code"
)]
struct Cli {
    /// Configuration file to load.
    #[arg(long, global = true, default_value = "theresa.toml")]
    config: PathBuf,

    /// Increase log detail. Can be repeated.
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Watch configured projects until interrupted (the default command).
    Watch,
    /// Inspect projects and update their READMEs once.
    Once {
        /// Zero-based project index from the config.
        #[arg(long)]
        project: Option<usize>,
        /// Report whether output would change without writing it.
        #[arg(long)]
        dry_run: bool,
    },
    /// Write a documented starter configuration.
    Init {
        /// Directories to monitor; defaults to the current directory.
        paths: Vec<PathBuf>,
        /// Replace an existing configuration file.
        #[arg(long)]
        force: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    match cli.command.unwrap_or(Command::Watch) {
        Command::Init { mut paths, force } => {
            if paths.is_empty() {
                paths.push(PathBuf::from("."));
            }
            if cli.config.exists() && !force {
                bail!(
                    "{} already exists; pass --force to replace it",
                    cli.config.display()
                );
            }
            let config = theresa::Config::template(&paths);
            let rendered = toml::to_string_pretty(&config)?;
            std::fs::write(&cli.config, rendered)
                .with_context(|| format!("failed to write {}", cli.config.display()))?;
            println!("created {}", cli.config.display());
        }
        Command::Once { project, dry_run } => {
            let config = theresa::Config::load(&cli.config)?;
            if let Some(index) = project {
                if index >= config.project.len() {
                    bail!(
                        "project index {index} is out of range; config has {} projects",
                        config.project.len()
                    );
                }
            }
            let changed = theresa::run_once(&config, project, dry_run).await?;
            if dry_run {
                println!("{changed} README(s) would change");
            } else {
                println!("updated {changed} README(s)");
            }
        }
        Command::Watch => {
            let config = theresa::Config::load(&cli.config)?;
            theresa::watch(config).await?;
        }
    }
    Ok(())
}

fn init_tracing(verbose: u8) {
    let fallback = match verbose {
        0 => "theresa=info",
        1 => "theresa=debug",
        _ => "theresa=trace,notify=debug",
    };
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(fallback));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}
