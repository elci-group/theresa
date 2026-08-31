use crate::scan::ProjectSnapshot;
use anyhow::{Context, Result};
use std::path::Path;

const BEGIN: &str = "<!-- theresa:begin -->";
const END: &str = "<!-- theresa:end -->";

pub fn render(snapshot: &ProjectSnapshot) -> String {
    let mut out = String::new();
    out.push_str(BEGIN);
    out.push_str("\n\n# ");
    out.push_str(&snapshot.name);
    out.push_str("\n\n");
    out.push_str(&snapshot.description);
    out.push_str("\n\n> This section is maintained automatically by [Theresa](https://github.com/elci-group/scotia/tree/main/theresa).\n");

    out.push_str("\n## Overview\n\n");
    out.push_str(&format!(
        "The repository currently contains {} inspected {}",
        snapshot.file_count,
        if snapshot.file_count == 1 {
            "file"
        } else {
            "files"
        }
    ));
    if let Some(version) = &snapshot.version {
        out.push_str(&format!(" and reports version `{version}`"));
    }
    out.push_str(". The summary below is derived from manifests and source layout, so it stays aligned as the project evolves.\n");

    out.push_str("\n## Highlights\n\n");
    for highlight in &snapshot.highlights {
        out.push_str("- ");
        out.push_str(highlight);
        out.push('\n');
    }

    if !snapshot.languages.is_empty() {
        out.push_str("\n## Technology\n\n");
        for language in &snapshot.languages {
            out.push_str(&format!(
                "- **{}:** {} source {}\n",
                language.name,
                language.files,
                if language.files == 1 { "file" } else { "files" }
            ));
        }
    }

    if !snapshot.structure.is_empty() {
        out.push_str("\n## Project layout\n\n```text\n");
        for entry in &snapshot.structure {
            out.push_str(entry);
            out.push('\n');
        }
        out.push_str("```\n");
    }

    out.push_str("\n## Getting started\n\n### Prerequisites\n\n");
    for prerequisite in &snapshot.prerequisites {
        out.push_str("- ");
        out.push_str(prerequisite);
        out.push('\n');
    }
    if !snapshot.commands.is_empty() {
        out.push_str("\n### Common commands\n\n");
        for command in &snapshot.commands {
            out.push_str(&format!(
                "**{}**\n\n```sh\n{}\n```\n\n",
                command.label, command.command
            ));
        }
    }

    out.push_str("## Development\n\n");
    if snapshot.has_tests {
        out.push_str("Run the test command before submitting changes. Keep tests close to the behavior they cover and update them with production code.\n");
    } else {
        out.push_str("Follow the existing source layout and add automated coverage when introducing behavior that can regress.\n");
    }

    if let Some(license) = &snapshot.license {
        out.push_str(&format!(
            "\n## License\n\nSee [`{license}`](./{license}) for licensing terms.\n"
        ));
    }
    if snapshot.truncated {
        out.push_str("\n_The source inventory reached Theresa's configured scan limit; counts may be partial._\n");
    }
    out.push('\n');
    out.push_str(END);
    out.push('\n');
    out
}

pub fn merge(existing: &str, generated: &str) -> String {
    match (existing.find(BEGIN), existing.find(END)) {
        (Some(begin), Some(end)) if begin <= end => {
            let after = end + END.len();
            format!(
                "{}{}{}",
                &existing[..begin],
                generated.trim_end(),
                &existing[after..]
            )
        }
        _ if existing.trim().is_empty() => generated.to_string(),
        _ => format!(
            "{}\n\n## Additional documentation\n\n{}",
            generated.trim_end(),
            existing.trim_start()
        ),
    }
}

pub fn write_atomic(path: &Path, generated: &str, dry_run: bool) -> Result<bool> {
    let existing = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()))
        }
    };
    let merged = merge(&existing, generated);
    if merged == existing {
        return Ok(false);
    }
    if dry_run {
        return Ok(true);
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("README.md");
    let temporary = parent.join(format!(".{file_name}.theresa-{}.tmp", std::process::id()));
    std::fs::write(&temporary, merged)
        .with_context(|| format!("failed to write temporary README {}", temporary.display()))?;
    if let Err(error) = std::fs::rename(&temporary, path) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error).with_context(|| format!("failed to replace {}", path.display()));
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_only_the_managed_section() {
        let old = "intro\n<!-- theresa:begin -->\nold\n<!-- theresa:end -->\nmanual\n";
        let new = "<!-- theresa:begin -->\nnew\n<!-- theresa:end -->\n";
        let merged = merge(old, new);
        assert_eq!(
            merged,
            "intro\n<!-- theresa:begin -->\nnew\n<!-- theresa:end -->\nmanual\n"
        );
    }

    #[test]
    fn preserves_an_unmanaged_readme() {
        let merged = merge(
            "# Handwritten\n\nKeep me.\n",
            "<!-- theresa:begin -->\nGenerated\n<!-- theresa:end -->\n",
        );
        assert!(merged.contains("## Additional documentation"));
        assert!(merged.contains("# Handwritten"));
    }
}
