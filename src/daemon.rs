use crate::batching::Batcher;
use crate::config::{Config, ProjectConfig, ScanConfig};
use crate::{readme, scan};
use anyhow::{Context, Result};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;

struct WatchEvent {
    project: usize,
    event: notify::Result<Event>,
}

pub async fn run_once(config: &Config, selected: Option<usize>, dry_run: bool) -> Result<usize> {
    let mut changed = 0;
    for (index, project) in config.project.iter().enumerate() {
        if selected.is_some_and(|selected| selected != index) {
            continue;
        }
        if generate(project, &config.scan, dry_run).await? {
            changed += 1;
        }
    }
    Ok(changed)
}

pub async fn watch(config: Config) -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut watchers = Vec::with_capacity(config.project.len());

    for (index, project) in config.project.iter().enumerate() {
        let event_tx = tx.clone();
        let mut watcher: RecommendedWatcher = notify::recommended_watcher(move |event| {
            let _ = event_tx.send(WatchEvent {
                project: index,
                event,
            });
        })
        .with_context(|| format!("failed to create watcher for {}", project.path.display()))?;
        let mode = if config.watch.recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };
        watcher
            .watch(&project.path, mode)
            .with_context(|| format!("failed to watch {}", project.path.display()))?;
        tracing::info!(project = index, path = %project.path.display(), "watching project");
        watchers.push(watcher);
    }
    drop(tx);

    if config.watch.rescan_on_start {
        let changed = run_once(&config, None, false).await?;
        tracing::info!(changed, "startup reconciliation complete");
    }

    let mut batcher = Batcher::new(
        Duration::from_millis(config.watch.settle_ms),
        Duration::from_millis(config.watch.max_settle_ms),
        config.watch.burst_threshold,
    );
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            maybe_event = rx.recv() => {
                let Some(watch_event) = maybe_event else {
                    break;
                };
                match watch_event.event {
                    Ok(event) if is_change(&event.kind) => {
                        let project = &config.project[watch_event.project];
                        let relevant = relevant_paths(project, event.paths)?;
                        if !relevant.is_empty() {
                            tracing::debug!(project = watch_event.project, paths = ?relevant, "code change detected");
                            batcher.ingest(watch_event.project, relevant);
                        }
                    }
                    Ok(_) => {}
                    Err(error) => tracing::warn!(project = watch_event.project, %error, "filesystem watcher error"),
                }
            }
            _ = tick.tick() => {
                for batch in batcher.drain_ready() {
                    process_batch(&config, batch).await;
                }
            }
            signal = tokio::signal::ctrl_c() => {
                signal.context("failed to listen for shutdown signal")?;
                tracing::info!("shutdown requested; flushing pending changes");
                for batch in batcher.drain_all() {
                    process_batch(&config, batch).await;
                }
                break;
            }
        }
    }

    drop(watchers);
    Ok(())
}

async fn process_batch(config: &Config, batch: crate::batching::ReadyBatch) {
    let project = &config.project[batch.project];
    tracing::info!(
        project = batch.project,
        events = batch.events,
        paths = batch.paths.len(),
        "processing code change burst"
    );
    match generate(project, &config.scan, false).await {
        Ok(true) => tracing::info!(path = %project.readme_path().display(), "README updated"),
        Ok(false) => {
            tracing::debug!(path = %project.readme_path().display(), "README already current")
        }
        Err(error) => {
            tracing::error!(path = %project.path.display(), %error, "README generation failed")
        }
    }
}

async fn generate(
    project: &ProjectConfig,
    scan_config: &ScanConfig,
    dry_run: bool,
) -> Result<bool> {
    let project = project.clone();
    let scan_config = scan_config.clone();
    tokio::task::spawn_blocking(move || {
        let snapshot = scan::inspect(&project, &scan_config)
            .with_context(|| format!("failed to inspect {}", project.path.display()))?;
        let generated = readme::render(&snapshot);
        readme::write_atomic(&project.readme_path(), &generated, dry_run)
    })
    .await
    .context("project inspection task panicked")?
}

fn relevant_paths(project: &ProjectConfig, paths: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    let matcher = scan::build_matcher(project)?;
    let readme = project.readme_path();
    Ok(paths
        .into_iter()
        .filter(|path| path != &readme)
        .filter(|path| {
            let relative = path.strip_prefix(&project.path).unwrap_or(path);
            !matcher.is_match(relative) && scan::is_code_related(relative)
        })
        .collect())
}

fn is_change(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_)
    )
}
