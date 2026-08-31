# Theresa

Theresa is a quiet, source-aware daemon that keeps `README.md` files aligned with the projects they describe. Point one Theresa process at one or more directories; it watches for meaningful code and manifest changes, batches noisy editor events, inspects each affected project, and atomically refreshes a managed README section.

It is deliberately deterministic. README generation does not require an API key, network access, or an opaque model call, so the same tree produces the same documentation.

## Why Theresa?

READMEs tend to drift because updating documentation is separate from changing code. Theresa closes that gap without rewriting every file event or taking ownership of hand-written notes.

- Monitors several project roots from one long-running process.
- Uses the platform's native filesystem notification backend through `notify`.
- Reacts only to source files, manifests, lockfiles, and build configuration.
- Coalesces save storms with adaptive quiet periods and a hard maximum delay.
- Reconciles missed changes on startup.
- Ignores build outputs, dependency trees, configured globs, and its own README writes.
- Derives names, descriptions, versions, languages, commands, tests, layout, and licensing from the repository.
- Preserves all content outside `theresa:begin` and `theresa:end` markers.
- Writes through a same-directory temporary file and rename, preventing partial READMEs.
- Shuts down cleanly on `Ctrl-C` and flushes any pending change burst.

## Installation

Theresa currently builds from source and requires a current stable Rust toolchain.

```sh
git clone git@github.com:elci-group/scotia.git
cd scotia/theresa
cargo build --release
install -Dm755 target/release/theresa "$HOME/.local/bin/theresa"
```

To work on Theresa itself:

```sh
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
```

## Quick start

Create a configuration for the current project:

```sh
theresa init .
```

Preview whether generation would change a README:

```sh
theresa once --dry-run
```

Generate immediately, then start the foreground daemon:

```sh
theresa once
theresa watch
```

`watch` is the default command, so `theresa` is equivalent to `theresa watch`. Use `-v` or `-vv` for progressively more detailed event logging. `RUST_LOG` overrides the built-in log filter.

## Configuration

Theresa reads `theresa.toml` from the working directory by default. Pass a different path with `--config`. Relative project paths are resolved against the configuration file, which makes service deployments predictable.

```toml
[watch]
recursive = true
rescan_on_start = true
settle_ms = 900
max_settle_ms = 8000
burst_threshold = 20

[scan]
max_files = 10000
max_structure_entries = 14

[[project]]
path = "../api"
readme = "README.md"
ignore = ["fixtures/**", "src/generated/**"]

[[project]]
path = "../web"
readme = "docs/README.md"
name = "Web Console"
description = "The browser interface for the platform."
ignore = ["public/vendor/**"]
```

The complete starter file is available at [`theresa.toml.example`](./theresa.toml.example).

### Watch settings

| Setting | Default | Purpose |
| --- | ---: | --- |
| `recursive` | `true` | Observe each complete project tree. |
| `rescan_on_start` | `true` | Generate once at startup, covering changes made while Theresa was stopped. |
| `settle_ms` | `900` | Base quiet period before processing a change burst. |
| `max_settle_ms` | `8000` | Hard deadline that prevents constant writes from starving generation. |
| `burst_threshold` | `20` | Event count at which the adaptive quiet period reaches its maximum growth. |

### Scan settings

| Setting | Default | Purpose |
| --- | ---: | --- |
| `max_files` | `10000` | Bound a project scan so unexpectedly large trees remain responsive. |
| `max_structure_entries` | `14` | Limit the top-level layout shown in generated documentation. |

Each `[[project]]` needs a `path`. `readme` defaults to `README.md`; `name` and `description` override discovered package metadata. Ignore entries use glob syntax and are applied in addition to built-in exclusions for `.git`, `.theresa`, `target`, `node_modules`, `.venv`, `dist`, and `build` directories.

## How it works

Theresa borrows the operational lessons that make Kaptaind's repository watcher dependable:

1. A native recursive watcher emits filesystem events into an unbounded handoff channel, keeping the callback short.
2. Events are mapped to their configured project, filtered against ignore rules, and classified as code-related.
3. A batcher deduplicates paths and waits for a quiet period. Bursty activity extends that period, while `max_settle_ms` guarantees progress.
4. The project scan runs on a blocking worker so filesystem traversal never stalls the async control loop.
5. Package manifests and the bounded source inventory become a deterministic project snapshot.
6. Theresa renders the snapshot, replaces only its managed section, compares the result, and skips a write when nothing changed.
7. A changed README is written to a temporary sibling and atomically renamed into place. Its path is excluded from subsequent events, avoiding feedback loops.

On shutdown, Theresa drains pending batches before dropping its watchers. Individual generation failures are logged and isolated; the daemon continues monitoring the other projects.

## Managed README sections

Generated content lives between these markers:

```html
<!-- theresa:begin -->
...generated documentation...
<!-- theresa:end -->
```

On the first run:

- An empty or absent README becomes a fully generated README.
- An existing unmarked README is preserved under an `Additional documentation` heading after the generated section.

On later runs only the marked region changes. You can keep architecture notes, deployment runbooks, screenshots, and other hand-written material before or after it.

## Running under systemd

Install the binary and configuration first, then create `/etc/systemd/system/theresa.service`:

```ini
[Unit]
Description=Theresa README maintenance daemon
After=local-fs.target

[Service]
Type=simple
User=theresa
Group=theresa
WorkingDirectory=/srv/projects
ExecStart=/usr/local/bin/theresa --config /etc/theresa.toml watch
Restart=on-failure
RestartSec=3
Environment=RUST_LOG=theresa=info
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=read-only
ReadWritePaths=/srv/projects

[Install]
WantedBy=multi-user.target
```

Then enable it:

```sh
sudo systemctl daemon-reload
sudo systemctl enable --now theresa.service
journalctl -u theresa.service -f
```

Adjust `User`, paths, and `ReadWritePaths` for the directories in your configuration. Theresa only needs read access to project trees and write access to the configured README destinations.

## Command reference

```text
theresa [OPTIONS] [COMMAND]

Commands:
  watch    Watch configured projects until interrupted
  once     Inspect and update projects once
  init     Write a starter configuration

Global options:
  --config <PATH>   Configuration file [default: theresa.toml]
  -v, --verbose     Increase log detail; repeat for trace output
  -h, --help        Show help
  -V, --version     Show version
```

Use `theresa once --project 1` to process only the second `[[project]]` entry, or `theresa init --force .` to intentionally replace an existing starter config.

## Scope and safety

Theresa documents observable repository facts; it does not attempt to infer product claims, security guarantees, or operational promises that source structure cannot prove. It does not execute project code or lifecycle scripts. Malformed manifests and invalid ignore patterns fail with context instead of silently producing misleading output.

Symlinks are not followed during scanning. Very large repositories are bounded by `max_files`, and generated output says when counts may be partial.

## License

Theresa is available under the MIT License. See [`LICENSE`](./LICENSE).

