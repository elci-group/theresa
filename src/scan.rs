use crate::config::{ProjectConfig, ScanConfig};
use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde_json::Value as JsonValue;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use walkdir::{DirEntry, WalkDir};

const DEFAULT_IGNORES: &[&str] = &[
    ".git",
    ".git/**",
    ".theresa",
    ".theresa/**",
    "target",
    "target/**",
    "**/target",
    "**/target/**",
    "node_modules",
    "node_modules/**",
    "**/node_modules",
    "**/node_modules/**",
    ".venv",
    ".venv/**",
    "**/.venv",
    "**/.venv/**",
    "dist",
    "dist/**",
    "build",
    "build/**",
];

const CODE_EXTENSIONS: &[(&str, &str)] = &[
    ("rs", "Rust"),
    ("ts", "TypeScript"),
    ("tsx", "TypeScript"),
    ("js", "JavaScript"),
    ("jsx", "JavaScript"),
    ("py", "Python"),
    ("go", "Go"),
    ("java", "Java"),
    ("kt", "Kotlin"),
    ("swift", "Swift"),
    ("c", "C"),
    ("h", "C"),
    ("cpp", "C++"),
    ("cc", "C++"),
    ("hpp", "C++"),
    ("cs", "C#"),
    ("rb", "Ruby"),
    ("php", "PHP"),
    ("ex", "Elixir"),
    ("exs", "Elixir"),
    ("scala", "Scala"),
    ("sh", "Shell"),
    ("sql", "SQL"),
    ("vue", "Vue"),
    ("svelte", "Svelte"),
];

const MANIFESTS: &[&str] = &[
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "go.mod",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "Makefile",
    "Dockerfile",
    "compose.yaml",
    "docker-compose.yml",
];

#[derive(Debug, Clone)]
pub struct LanguageStat {
    pub name: String,
    pub files: usize,
}

#[derive(Debug, Clone)]
pub struct CommandHint {
    pub label: String,
    pub command: String,
}

#[derive(Debug, Clone)]
pub struct ProjectSnapshot {
    pub name: String,
    pub description: String,
    pub version: Option<String>,
    pub languages: Vec<LanguageStat>,
    pub file_count: usize,
    pub structure: Vec<String>,
    pub highlights: Vec<String>,
    pub prerequisites: Vec<String>,
    pub commands: Vec<CommandHint>,
    pub has_tests: bool,
    pub license: Option<String>,
    pub truncated: bool,
}

pub fn inspect(project: &ProjectConfig, scan: &ScanConfig) -> Result<ProjectSnapshot> {
    let matcher = build_matcher(project)?;
    let mut language_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut files = Vec::new();
    let mut truncated = false;

    let walker = WalkDir::new(&project.path)
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| should_descend(entry, &project.path, &matcher));

    for entry in walker {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                tracing::debug!(%error, "skipping unreadable project entry");
                continue;
            }
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(&project.path)
            .unwrap_or(entry.path());
        if matcher.is_match(relative) || relative == project.readme {
            continue;
        }
        files.push(relative.to_path_buf());
        if let Some(language) = language_for(relative) {
            *language_counts.entry(language.to_string()).or_default() += 1;
        }
        if files.len() >= scan.max_files {
            truncated = true;
            break;
        }
    }

    files.sort();
    let metadata = metadata(&project.path)?;
    let name = project.name.clone().unwrap_or_else(|| {
        metadata.name.unwrap_or_else(|| {
            project
                .path
                .file_name()
                .and_then(|name| name.to_str())
                .map(title_case)
                .unwrap_or_else(|| "Project".to_string())
        })
    });
    let description = project
        .description
        .clone()
        .or(metadata.description)
        .unwrap_or_else(|| inferred_description(&name, &language_counts));

    let languages: Vec<_> = {
        let mut stats: Vec<_> = language_counts
            .into_iter()
            .map(|(name, files)| LanguageStat { name, files })
            .collect();
        stats.sort_by(|a, b| b.files.cmp(&a.files).then_with(|| a.name.cmp(&b.name)));
        stats
    };
    let has_tests = files.iter().any(|path| is_test_path(path));
    let commands = command_hints(&project.path, &metadata.package_scripts);
    let prerequisites = prerequisite_hints(&project.path);
    let structure = structure(&project.path, &matcher, scan.max_structure_entries)?;
    let highlights = highlights(&project.path, &languages, has_tests, truncated);
    let license = detect_license(&project.path);

    Ok(ProjectSnapshot {
        name,
        description,
        version: metadata.version,
        languages,
        file_count: files.len(),
        structure,
        highlights,
        prerequisites,
        commands,
        has_tests,
        license,
        truncated,
    })
}

pub fn is_code_related(path: &Path) -> bool {
    if language_for(path).is_some() {
        return true;
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    MANIFESTS.contains(&name)
        || matches!(
            name,
            "Cargo.lock"
                | "package-lock.json"
                | "pnpm-lock.yaml"
                | "yarn.lock"
                | "uv.lock"
                | "poetry.lock"
                | "go.sum"
                | ".env.example"
                | "theresa.toml"
        )
}

pub fn build_matcher(project: &ProjectConfig) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in DEFAULT_IGNORES
        .iter()
        .copied()
        .chain(project.ignore.iter().map(String::as_str))
    {
        builder.add(
            Glob::new(pattern).with_context(|| format!("invalid ignore pattern {pattern:?}"))?,
        );
    }
    builder.build().context("failed to compile ignore patterns")
}

fn should_descend(entry: &DirEntry, root: &Path, matcher: &GlobSet) -> bool {
    if entry.depth() == 0 {
        return true;
    }
    let relative = entry.path().strip_prefix(root).unwrap_or(entry.path());
    !matcher.is_match(relative)
}

fn language_for(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    CODE_EXTENSIONS
        .iter()
        .find_map(|(ext, language)| (*ext == extension).then_some(*language))
}

#[derive(Default)]
struct Metadata {
    name: Option<String>,
    description: Option<String>,
    version: Option<String>,
    package_scripts: BTreeSet<String>,
}

fn metadata(root: &Path) -> Result<Metadata> {
    if root.join("Cargo.toml").is_file() {
        let raw = std::fs::read_to_string(root.join("Cargo.toml"))?;
        let value: toml::Value = toml::from_str(&raw)?;
        let package = value.get("package");
        return Ok(Metadata {
            name: package
                .and_then(|p| p.get("name"))
                .and_then(toml::Value::as_str)
                .map(str::to_string),
            description: package
                .and_then(|p| p.get("description"))
                .and_then(toml::Value::as_str)
                .map(str::to_string),
            version: package
                .and_then(|p| p.get("version"))
                .and_then(toml::Value::as_str)
                .map(str::to_string),
            ..Metadata::default()
        });
    }
    if root.join("package.json").is_file() {
        let raw = std::fs::read_to_string(root.join("package.json"))?;
        let value: JsonValue = serde_json::from_str(&raw)?;
        let scripts = value
            .get("scripts")
            .and_then(JsonValue::as_object)
            .map(|scripts| scripts.keys().cloned().collect())
            .unwrap_or_default();
        return Ok(Metadata {
            name: json_string(&value, "name"),
            description: json_string(&value, "description"),
            version: json_string(&value, "version"),
            package_scripts: scripts,
        });
    }
    if root.join("pyproject.toml").is_file() {
        let raw = std::fs::read_to_string(root.join("pyproject.toml"))?;
        let value: toml::Value = toml::from_str(&raw)?;
        let package = value
            .get("project")
            .or_else(|| value.get("tool").and_then(|v| v.get("poetry")));
        return Ok(Metadata {
            name: package
                .and_then(|p| p.get("name"))
                .and_then(toml::Value::as_str)
                .map(str::to_string),
            description: package
                .and_then(|p| p.get("description"))
                .and_then(toml::Value::as_str)
                .map(str::to_string),
            version: package
                .and_then(|p| p.get("version"))
                .and_then(toml::Value::as_str)
                .map(str::to_string),
            ..Metadata::default()
        });
    }
    Ok(Metadata::default())
}

fn json_string(value: &JsonValue, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(JsonValue::as_str)
        .map(str::to_string)
}

fn title_case(value: &str) -> String {
    value
        .split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn inferred_description(name: &str, languages: &BTreeMap<String, usize>) -> String {
    match languages.iter().max_by_key(|(_, count)| *count) {
        Some((language, _)) => format!("{name} is a {language} project."),
        None => format!("Documentation and development guide for {name}."),
    }
}

fn is_test_path(path: &Path) -> bool {
    path.components().any(|part| {
        matches!(
            part.as_os_str().to_str(),
            Some("test" | "tests" | "spec" | "specs" | "__tests__")
        )
    }) || path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .is_some_and(|stem| {
            stem.ends_with("_test") || stem.ends_with(".test") || stem.ends_with(".spec")
        })
}

fn structure(root: &Path, matcher: &GlobSet, limit: usize) -> Result<Vec<String>> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(root).context("failed to read project root")? {
        let entry = entry?;
        let relative = PathBuf::from(entry.file_name());
        if matcher.is_match(&relative) || relative == Path::new("README.md") {
            continue;
        }
        let file_type = entry.file_type()?;
        let suffix = if file_type.is_dir() { "/" } else { "" };
        entries.push(format!("{}{suffix}", relative.display()));
    }
    entries.sort();
    entries.truncate(limit);
    Ok(entries)
}

fn highlights(
    root: &Path,
    languages: &[LanguageStat],
    has_tests: bool,
    truncated: bool,
) -> Vec<String> {
    let mut result = Vec::new();
    if let Some(primary) = languages.first() {
        result.push(format!(
            "Primarily {} ({} source {}).",
            primary.name,
            primary.files,
            if primary.files == 1 { "file" } else { "files" }
        ));
    }
    if languages.len() > 1 {
        result.push(format!(
            "Uses {} implementation languages.",
            languages.len()
        ));
    }
    if has_tests {
        result.push("Includes an automated test suite.".to_string());
    }
    if root.join("Dockerfile").is_file() || root.join("compose.yaml").is_file() {
        result.push("Includes container-based development or deployment support.".to_string());
    }
    if root.join(".github/workflows").is_dir() {
        result.push("Includes GitHub Actions automation.".to_string());
    }
    if truncated {
        result.push("Uses a bounded source scan to remain responsive on large trees.".to_string());
    }
    if result.is_empty() {
        result
            .push("Keeps its source and project metadata together in one repository.".to_string());
    }
    result
}

fn prerequisite_hints(root: &Path) -> Vec<String> {
    let mut hints = Vec::new();
    if root.join("Cargo.toml").is_file() {
        hints.push("A current stable Rust toolchain (`rustup` recommended)".to_string());
    }
    if root.join("package.json").is_file() {
        hints.push("Node.js and the package manager matching the lockfile".to_string());
    }
    if root.join("pyproject.toml").is_file() {
        hints.push("Python 3 and a virtual environment".to_string());
    }
    if root.join("go.mod").is_file() {
        hints.push("The Go toolchain version declared by `go.mod`".to_string());
    }
    if hints.is_empty() {
        hints.push("The toolchain required by the source files in this repository".to_string());
    }
    hints
}

fn command_hints(root: &Path, scripts: &BTreeSet<String>) -> Vec<CommandHint> {
    if root.join("Cargo.toml").is_file() {
        return vec![
            hint("Build", "cargo build"),
            hint("Run", "cargo run"),
            hint("Test", "cargo test"),
        ];
    }
    if root.join("package.json").is_file() {
        let package_manager = if root.join("pnpm-lock.yaml").is_file() {
            "pnpm"
        } else if root.join("yarn.lock").is_file() {
            "yarn"
        } else {
            "npm"
        };
        let mut commands = vec![hint(
            "Install dependencies",
            &format!("{package_manager} install"),
        )];
        for (label, script) in [
            ("Run", "start"),
            ("Develop", "dev"),
            ("Build", "build"),
            ("Test", "test"),
        ] {
            if scripts.contains(script) {
                let command = if package_manager == "npm" {
                    format!("npm run {script}")
                } else {
                    format!("{package_manager} {script}")
                };
                commands.push(hint(label, &command));
            }
        }
        return commands;
    }
    if root.join("pyproject.toml").is_file() {
        return vec![
            hint("Create an environment", "python -m venv .venv"),
            hint("Install", "python -m pip install -e ."),
            hint("Test", "pytest"),
        ];
    }
    if root.join("go.mod").is_file() {
        return vec![hint("Run", "go run ."), hint("Test", "go test ./...")];
    }
    if root.join("Makefile").is_file() {
        return vec![hint("Build", "make"), hint("Test", "make test")];
    }
    Vec::new()
}

fn hint(label: &str, command: &str) -> CommandHint {
    CommandHint {
        label: label.to_string(),
        command: command.to_string(),
    }
}

fn detect_license(root: &Path) -> Option<String> {
    let names = ["LICENSE", "LICENSE.md", "LICENSE.txt", "COPYING"];
    names
        .iter()
        .find(|name| root.join(name).is_file())
        .map(|name| (*name).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn inspects_a_rust_project_without_build_artifacts() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::create_dir_all(temp.path().join("target/debug")).unwrap();
        fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname='little-engine'\nversion='1.2.3'\ndescription='Small and fast'\n",
        )
        .unwrap();
        fs::write(temp.path().join("src/lib.rs"), "pub fn run() {}\n").unwrap();
        fs::write(temp.path().join("target/debug/generated.rs"), "bad\n").unwrap();
        let project = ProjectConfig {
            path: temp.path().to_path_buf(),
            readme: PathBuf::from("README.md"),
            name: None,
            description: None,
            ignore: Vec::new(),
        };
        let snapshot = inspect(&project, &ScanConfig::default()).unwrap();
        assert_eq!(snapshot.name, "little-engine");
        assert_eq!(snapshot.version.as_deref(), Some("1.2.3"));
        assert_eq!(snapshot.languages[0].files, 1);
        assert!(!snapshot.structure.iter().any(|entry| entry == "target/"));
    }

    #[test]
    fn identifies_code_and_manifest_changes() {
        assert!(is_code_related(Path::new("src/main.rs")));
        assert!(is_code_related(Path::new("package.json")));
        assert!(!is_code_related(Path::new("notes.md")));
    }
}
