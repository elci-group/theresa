use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub watch: WatchConfig,
    #[serde(default)]
    pub scan: ScanConfig,
    pub project: Vec<ProjectConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct WatchConfig {
    pub recursive: bool,
    pub rescan_on_start: bool,
    pub settle_ms: u64,
    pub max_settle_ms: u64,
    pub burst_threshold: usize,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            recursive: true,
            rescan_on_start: true,
            settle_ms: 900,
            max_settle_ms: 8_000,
            burst_threshold: 20,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ScanConfig {
    pub max_files: usize,
    pub max_structure_entries: usize,
}

impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            max_files: 10_000,
            max_structure_entries: 14,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    pub path: PathBuf,
    #[serde(default = "default_readme")]
    pub readme: PathBuf,
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub ignore: Vec<String>,
}

fn default_readme() -> PathBuf {
    PathBuf::from("README.md")
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config at {}", path.display()))?;
        let mut config: Self = toml::from_str(&raw)
            .with_context(|| format!("failed to parse config at {}", path.display()))?;
        config.resolve_paths(path)?;
        config.validate()?;
        Ok(config)
    }

    fn resolve_paths(&mut self, config_path: &Path) -> Result<()> {
        let base = config_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .canonicalize()
            .with_context(|| {
                format!(
                    "failed to resolve config directory for {}",
                    config_path.display()
                )
            })?;

        for project in &mut self.project {
            if project.path.is_relative() {
                project.path = base.join(&project.path);
            }
            project.path = project.path.canonicalize().with_context(|| {
                format!("project path does not exist: {}", project.path.display())
            })?;
        }
        Ok(())
    }

    fn validate(&self) -> Result<()> {
        if self.project.is_empty() {
            bail!("config must contain at least one [[project]] entry");
        }
        if self.watch.settle_ms == 0 {
            bail!("watch.settle_ms must be greater than zero");
        }
        if self.watch.max_settle_ms < self.watch.settle_ms {
            bail!("watch.max_settle_ms must be at least watch.settle_ms");
        }
        if self.watch.burst_threshold == 0 {
            bail!("watch.burst_threshold must be greater than zero");
        }
        if self.scan.max_files == 0 {
            bail!("scan.max_files must be greater than zero");
        }
        for project in &self.project {
            if !project.path.is_dir() {
                bail!(
                    "project path is not a directory: {}",
                    project.path.display()
                );
            }
            if project.readme.is_absolute() {
                bail!(
                    "project readme must be relative to its project: {}",
                    project.readme.display()
                );
            }
        }
        Ok(())
    }

    pub fn template(paths: &[PathBuf]) -> Self {
        let projects = paths
            .iter()
            .map(|path| ProjectConfig {
                path: path.clone(),
                readme: default_readme(),
                name: None,
                description: None,
                ignore: Vec::new(),
            })
            .collect();
        Self {
            watch: WatchConfig::default(),
            scan: ScanConfig::default(),
            project: projects,
        }
    }
}

impl ProjectConfig {
    pub fn readme_path(&self) -> PathBuf {
        self.path.join(&self.readme)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_round_trips() {
        let config = Config::template(&[PathBuf::from(".")]);
        let text = toml::to_string_pretty(&config).unwrap();
        let decoded: Config = toml::from_str(&text).unwrap();
        assert_eq!(decoded.project.len(), 1);
        assert_eq!(decoded.watch.settle_ms, 900);
    }

    #[test]
    fn invalid_settle_window_is_rejected() {
        let mut config = Config::template(&[PathBuf::from(".")]);
        config.watch.max_settle_ms = 100;
        assert!(config.validate().is_err());
    }

    #[test]
    fn bare_config_path_resolves_relative_projects_from_current_directory() {
        let mut config = Config::template(&[PathBuf::from(".")]);
        config.resolve_paths(Path::new("theresa.toml")).unwrap();

        assert_eq!(config.project[0].path, std::env::current_dir().unwrap());
    }
}
