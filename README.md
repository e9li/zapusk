# zapusk

A lightweight terminal UI for managing local development projects. Built with Rust and Ratatui.

---

## What it is

`zapusk` solves a specific problem: when you work on many projects simultaneously across different stacks, keeping track of which project runs on which port, which PHP version it needs, and what its local domain is becomes a mess. You end up scanning a mental map of arbitrary port numbers and juggling terminal windows.

`zapusk` replaces that with a single TUI where every project has a meaningful `.test` domain, a human-readable name, and a clear running/stopped status. You start, stop and monitor all your projects from one place.

---

## What it manages

`zapusk` is designed around a stack of lightweight local tools:

| Tool | Role |
|------|------|
| **dnsmasq** | Wildcard `*.test` DNS — any `name.test` resolves to localhost automatically |
| **Caddy** | Reverse proxy — maps `name.test` → `localhost:PORT`, handles PHP-FPM for Kirby |
| **zapusk** | TUI — starts/stops project servers, streams logs, regenerates Caddyfile |

### Supported project types

| Type | How it runs |
|------|-------------|
| **Phoenix** | `mix phx.server` |
| **Symfony** | `symfony server:start` (reads `.php-version` if present) |
| **Kirby** | PHP-FPM via Caddy (Homebrew PHP, version per project) |
| **Axum** | `cargo run` |

The design is intentionally stack-agnostic — anything that binds to a port can be added as a project type. Caddy proxies it; `zapusk` manages the process.

---

## Why not Docker / ddev / Colima?

Those tools are great for team environments and production parity. For solo local development where your databases already run natively, the overhead is significant:

| | ddev + Colima | zapusk stack |
|---|---|---|
| RAM | ~500MB–1GB+ | ~50–80MB total |
| Startup | 10–30s (VM boot) | instant |
| Per-project PHP isolation | yes (containers) | yes (Homebrew PHP versions) |
| Complexity | high | low |

The trade-off is no container isolation. If that is acceptable for your workflow, this stack is much lighter.

---

## Architecture

```
~/.config/zapusk/
├── config.toml       ← project registry + caddy settings
└── Caddyfile         ← auto-generated, do not edit manually
```

```
src/
├── main.rs       # Entry point, terminal setup, render loop
├── app.rs        # App state, keyboard handling, orchestration
├── ui.rs         # Ratatui rendering (project list, log panel, status bar)
├── project.rs    # Project model (config + runtime state + log ring buffer)
├── manager.rs    # Child process spawning, stdout/stderr streaming via tokio
├── caddy.rs      # Caddyfile generation and `caddy reload`
├── config.rs     # TOML config deserialization
├── init.rs       # `zapusk init` — interactive first-run setup        [TODO]
└── doctor.rs     # `zapusk doctor` — dependency checks                [TODO]
```

---

## CLI subcommands

`zapusk` is invoked without arguments to open the TUI. It also supports subcommands:

```
zapusk              # open TUI
zapusk init         # interactive first-run setup
zapusk doctor       # check all dependencies
zapusk add          # add a project to config interactively    [TODO]
```

---

## `zapusk doctor`

Checks that everything required to run the full stack is present and correctly configured.
Should be runnable at any time, not just on first install.

### Checks to implement (`src/doctor.rs`)

#### System dependencies
- [ ] `caddy` binary in PATH — run `caddy version` and parse output
- [ ] `dnsmasq` installed — check via `which dnsmasq` or package manager
- [ ] dnsmasq config has `address=/.test/127.0.0.1` — read and scan config file
- [ ] dnsmasq is running — `systemctl is-active dnsmasq` (Linux) or `brew services list` (macOS)
- [ ] DNS resolves correctly — try resolving `zapusk-check.test` via system resolver

#### PHP (only checked if any Kirby/PHP projects exist in config)
- [ ] Each required PHP version installed at expected Homebrew path
- [ ] PHP-FPM service running for each required version
- [ ] FPM socket file exists and is accessible

#### Per-project checks
- [ ] Project `path` exists on disk
- [ ] For Phoenix: `mix` in PATH, `mix.exs` present in project path
- [ ] For Symfony: `symfony` CLI in PATH, `composer.json` present
- [ ] For Axum: `cargo` in PATH, `Cargo.toml` present
- [ ] Port not already occupied by another process

#### Caddy
- [ ] Caddyfile exists at configured path (or has been generated)
- [ ] `caddy validate --config <path>` passes

### Output format

```
zapusk doctor

System
  ✓ caddy 2.8.4
  ✓ dnsmasq installed
  ✓ dnsmasq running
  ✓ *.test resolves to 127.0.0.1

PHP
  ✓ php@8.1 found at /opt/homebrew/opt/php@8.1/bin/php
  ✓ php@8.3 found at /opt/homebrew/opt/php@8.3/bin/php
  ✓ php8.1-fpm running
  ✗ php8.3-fpm not running
    → run: brew services start php@8.3

Projects
  ✓ myshop       /home/user/projects/myshop (phoenix)
  ✓ company-site /home/user/projects/company-site (kirby@8.1)
  ✗ api          path not found: /home/user/projects/api
    → check the path in config.toml

Caddy
  ✓ Caddyfile present
  ✓ caddy validate passed

2 issues found. Run `zapusk init` to fix setup issues.
```

---

## `zapusk init`

Interactive first-run wizard. Guides the user through installing and configuring the full stack.
Should be idempotent — safe to re-run to fix a broken setup.

### Flow to implement (`src/init.rs`)

```
Welcome to zapusk!
Let us make sure your local dev stack is ready.

[1/5] Checking Caddy...
      ✓ caddy found (2.8.4)

[2/5] Checking dnsmasq...
      ✗ dnsmasq not found
      → Install dnsmasq? [Y/n]
        macOS:  brew install dnsmasq
        Linux:  sudo apt install dnsmasq
      Running: brew install dnsmasq ... done

[3/5] Configuring dnsmasq for *.test...
      ✓ address=/.test/127.0.0.1 already present

[4/5] Starting dnsmasq...
      → Start dnsmasq now? [Y/n]
        Running: brew services start dnsmasq ... done

[5/5] Generating Caddyfile from config...
      → Config found at ~/.config/zapusk/config.toml
      ✓ Caddyfile written to ~/.config/zapusk/Caddyfile
      → Reload Caddy? [Y/n]  done

Setup complete. Run `zapusk` to open the TUI.
```

### Init steps in detail

- Detect OS (macOS vs Linux) using `cfg!(target_os = "macos")` to pick correct install commands
- Check each dependency (reuse `doctor.rs` logic so checks are never duplicated)
- For missing tools: print the install command, optionally run it after user confirmation
- For dnsmasq config: locate the right file per OS, check for the `.test` entry, append if missing
- For macOS resolver: check `/etc/resolver/test` exists, create it if not (requires `sudo`)
- Write initial Caddyfile from current `config.toml`
- Run `caddy validate` before reloading
- At the end, run the full doctor check and show final status

---

## Keybinds (TUI)

| Key | Action |
|-----|--------|
| `s` | Start selected project |
| `x` | Stop selected project |
| `r` | Restart selected project |
| `R` | Regenerate Caddyfile + reload Caddy |
| `o` | Open project domain in browser |
| `tab` | Switch focus between project list and logs |
| `j/k` or `↑/↓` | Navigate project list |
| `q` | Quit (stops all running projects) |

---

## Config reference

```toml
# ~/.config/zapusk/config.toml

[[projects]]
name = "myshop"
domain = "myshop.test"
port = 4000
type = "phoenix"
path = "/home/user/projects/myshop"
autostart = false          # optional: start automatically on zapusk launch

[[projects]]
name = "company-site"
domain = "company.test"
port = 8001
type = "kirby"
php_version = "8.1"        # required for kirby — selects Homebrew PHP version
path = "/home/user/projects/company-site"

[[projects]]
name = "blog"
domain = "blog.test"
port = 8002
type = "symfony"
path = "/home/user/projects/blog"

[[projects]]
name = "api"
domain = "api.test"
port = 3000
type = "axum"
path = "/home/user/projects/api"

[caddy]
config_path = "/home/user/.config/zapusk/Caddyfile"
# caddy_bin = "caddy"      # optional, defaults to "caddy" from PATH
```

---

## TODO / Ideas for Claude Code

### High priority — core correctness
- [ ] `src/doctor.rs` — implement full dependency check (see spec above)
- [ ] `src/init.rs` — implement init wizard (see spec above)
- [ ] Wire subcommands in `main.rs` — parse `argv[1]` for `init` / `doctor` / `add`,
      fall through to TUI when no subcommand given
- [ ] Fix process exit detection in `manager.rs` — use `child.wait()` in a
      dedicated tokio task, send `ManagerEvent::ProcessExited` properly
- [ ] Detect port already in use before starting, show friendly error
- [ ] Parse `.php-version` file in Symfony project path for Symfony CLI compat

### UI improvements
- [ ] Scrollable log panel (scroll offset in App state, PageUp/PageDown/mouse wheel)
- [ ] Log search / filter (`/` to enter search mode, Esc to exit)
- [ ] Colour-code log lines by severity (scan for `error`, `warn`, `info` tokens)
- [ ] Show port + uptime in project list
- [ ] Popup/modal for project details (domain, path, pid, full command)
- [ ] Confirmation dialog before stopping a running project

### Config
- [ ] Watch config file for changes and hot-reload project list
- [ ] Support `env` map per project (forwarded to child process environment)
- [ ] Support `command` override per project (bypass built-in defaults)
- [ ] Make PHP-FPM socket path a config option (Linux paths differ from macOS Homebrew)

### Features
- [ ] `o` key: open project domain in browser (`open` on macOS, `xdg-open` on Linux)
- [ ] `c` key: copy domain to clipboard
- [ ] `autostart = true` in config: start those projects when zapusk launches
- [ ] Pidfile to detect projects left running from a previous session
- [ ] TLS support: `tls = true` per project — use `https://` in Caddyfile, run `caddy trust`

### CLI
- [ ] `zapusk add` — interactive prompt to append a project to config
- [ ] Shell completions (bash, zsh, fish) via `clap`
- [ ] Homebrew formula
