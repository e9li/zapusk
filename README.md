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
| **Caddy** | Reverse proxy — maps `name.test` → `localhost:PORT` for all project types |
| **zapusk** | TUI — starts/stops project servers, streams logs, regenerates Caddyfile |

### Supported project types

| Type | How it runs |
|------|-------------|
| **Phoenix** | `mix phx.server` |
| **Symfony** | `symfony server:start` (reads `.php-version` if present) |
| **Kirby** | PHP built-in server (`php -S`), proxied by Caddy (Homebrew PHP, version per project) |
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

## Framework notes

### Symfony: trusted proxies

Since Caddy reverse-proxies requests to the Symfony dev server, Symfony sees all requests as plain HTTP from localhost. Without trusted proxy configuration, `app.request.getSchemeAndHttpHost()` returns `http://` and the web debug toolbar may not appear.

Add to every Symfony project managed by zapusk:

1. In `.env`, uncomment or add:

```
TRUSTED_PROXIES=127.0.0.1
```

2. In `config/packages/framework.yaml`, add under `when@dev:` → `framework:`:

```yaml
when@dev:
    framework:
        trusted_proxies: '%env(TRUSTED_PROXIES)%'
        trusted_headers: ['x-forwarded-for', 'x-forwarded-host', 'x-forwarded-proto', 'x-forwarded-port']
```

This applies to both Symfony 7 and 8.

### Kirby: Caddy static files and base URL

Kirby projects get a special Caddy configuration: static files (images, CSS, JS, media) are served directly by Caddy with correct MIME types, while dynamic requests are proxied to PHP's built-in server using Kirby's own `kirby/router.php`.

Caddy also blocks access to sensitive directories (`/content/*`, `/site/*`, `/kirby/*`, `/.*`) at the reverse proxy level.

Since PHP's built-in server doesn't know it runs behind a TLS reverse proxy, Kirby may generate `http://` URLs by default. To fix this, set the base URL in your Kirby project config:

```php
// site/config/config.php
return [
    'url' => 'https://your-project.test',
];
```

This ensures all generated links (assets, media, panel) use the correct scheme and domain.

---

## Architecture

```
~/.config/zapusk/
├── config.toml       ← project registry + caddy settings
├── Caddyfile         ← auto-generated, do not edit manually
├── logs/             ← stdout/stderr log files per project (<name>.out, <name>.err)
└── pids/             ← pidfiles for process re-adoption across sessions (<name>.pid)
```

```
src/
├── main.rs           # Entry point, terminal setup, render loop
├── platform.rs       # All OS-specific logic (macOS vs Linux)
├── tui/
│   ├── app.rs        # App state, orchestration, process actions
│   ├── input.rs      # Keyboard handling (extracted from app)
│   └── ui.rs         # Ratatui rendering (project list, log panel, status bar)
├── core/
│   ├── project.rs    # Project model (config + runtime state + log ring buffer)
│   ├── manager.rs    # Child process spawning, log file tailing, pidfile tracking
│   ├── caddy.rs      # Caddyfile generation and `caddy reload`
│   ├── config.rs     # TOML config deserialization + ProjectType
│   └── discovery.rs  # Listening-port discovery + stack heuristics
└── cli/
    ├── doctor.rs     # `zapusk doctor` — dependency checks
    ├── init.rs       # `zapusk init` — interactive first-run setup
    ├── add.rs        # `zapusk add` — add project interactively
    ├── destroy.rs    # `zapusk destroy` — remove all zapusk configuration
    └── discover.rs   # `zapusk discover` — list unmanaged listening apps
```

---

## CLI subcommands

`zapusk` is invoked without arguments to open the TUI. It also supports subcommands:

```
zapusk              # open TUI
zapusk init         # interactive first-run setup
zapusk doctor       # check all dependencies
zapusk add          # add a project to config interactively
zapusk destroy      # remove all zapusk configuration
zapusk discover     # discover listening services (managed + unmanaged)
zapusk discover --import 4000  # import discovered service by port/pid
```

---

## Build and install locally (release)

Recommended local release workflow:

```bash
# from repository root
cargo build --release
mkdir -p "$HOME/.local/bin"
cp target/release/zapusk "$HOME/.local/bin/zapusk"
chmod +x "$HOME/.local/bin/zapusk"
```

Add to `~/.zshrc` (once):

```bash
export PATH="$HOME/.local/bin:$PATH"
```

Reload shell and verify:

```bash
source ~/.zshrc
which zapusk
zapusk --version
```

Alternative (Cargo-managed install path):

```bash
cargo install --path . --force
strip ~/.cargo/bin/zapusk
```

---

## `zapusk doctor`

Checks that everything required to run the full stack is present and correctly configured.
Should be runnable at any time, not just on first install.

### Checks performed

- **System:** caddy binary, dnsmasq installed/running/configured, DNS resolution
- **PHP:** per-version binary present (only if Kirby projects exist)
- **Projects:** path exists, expected files present, required binaries in PATH
- **Caddy:** Caddyfile exists, `caddy validate` passes

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

### Flow

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
| `x` | Stop selected project (with confirmation) |
| `r` | Restart selected project |
| `e` | Edit selected project |
| `D` or `Del` | Remove selected project from config |
| `R` | Regenerate Caddyfile + reload Caddy |
| `o` | Open project domain in browser |
| `c` | Copy domain to clipboard |
| `d` | Show project detail popup |
| `u` | Show unmanaged services popup |
| `/` | Search / filter logs |
| `tab` | Switch focus between project list and logs |
| `j/k` or `↑/↓` | Navigate project list |
| `PgUp/PgDn` | Scroll logs |
| `G` or `End` | Jump to latest logs |
| `q` | Quit (keeps running projects alive) |
| `Q` | Force quit (stops projects, then tries to stop Caddy/dnsmasq) |

Inside the unmanaged services popup (`u`): `j/k` select, `Enter` inspect,
`i` import as project, `I` ignore, `f` toggle stack filter (`dev-only`/`all`),
`w` toggle port filter (`web`/`all-ports`), `r` refresh, `Esc` close.

Left pane sections: Projects (top), Unmanaged summary (middle), Services health
(bottom: Caddy/dnsmasq as running/paused/stopped).

Project list badges: `[M]` = managed by zapusk, `[A]` = adopted external process.

Project list also shows `tls:on` / `tls:off` per project.

### Add/Edit form fields

Both Add (`a`) and Edit (`e`) include these fields:

- `Name`
- `Domain`
- `Port`
- `Upstream` (maps to `upstream_host`)
- `Type`
- `TLS` (`off`/`on`)
- `Directory`

Field behavior:

- text fields: type normally
- selector fields (`Type`, `TLS`): use `←/→` or `Tab`/`Shift+Tab`
- `Enter`: move to next field / submit at the end
- `Esc`: cancel

### Startup diagnostics in logs

When starting a project, zapusk writes diagnostic information directly to the project's log pane:

- `[zapusk] command: <bin> <args>` — the exact command that was launched
- `[zapusk] <note>` — any warnings before start (e.g. PHP binary fallback: `php@8.1: arm64 Homebrew path not found, using Intel path: ...`)
- `[zapusk] start failed: <error>` — if the binary is not found or not executable
- Caddy reload errors are also appended to the project log if Caddy fails to apply the new config

If the configured port is already in use, zapusk detects the existing process and
adopts it — either from a pidfile (previously managed by zapusk) or via `lsof` port
lookup. On the next `zapusk` start, pidfiles allow previously-running projects to be
re-adopted automatically without having been stopped.

---

## Config reference

```toml
# ~/.config/zapusk/config.toml

# tld = "test"            # optional: TLD for wildcard DNS (default: "test")

[[projects]]
name = "myshop"
domain = "myshop.test"
port = 4000
type = "phoenix"
path = "/home/user/projects/myshop"
autostart = false          # optional: start automatically on zapusk launch
tls = true                 # optional: enable https:// + `tls internal`

[[projects]]
name = "company-site"
domain = "company.test"
port = 8001
type = "kirby"
php_version = "8.1"        # required for kirby — selects Homebrew PHP version
path = "/home/user/projects/company-site"
# public_dir = "public"    # optional: document root subfolder (default: "public")

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
command = "cargo"         # optional: command override
args = ["run", "--bin", "api"]
upstream_host = "127.0.0.1" # optional: override reverse proxy target host

# if unset, zapusk uses loopback fallback for proxying:
#   127.0.0.1:<port> first, then [::1]:<port>

# `zapusk doctor` warns on duplicate ports and errors on duplicate upstream targets
# (same upstream_host + port across multiple projects).

[caddy]
config_path = "/home/user/.config/zapusk/Caddyfile"
# caddy_bin = "caddy"      # optional, defaults to "caddy" from PATH

[discovery]
# Optional: what `w` (web-only) means in unmanaged popup.
# Supports single ports and ranges.
web_ports = ["80", "443", "8080", "8443", "3000-9999"]

[[ignored_services]]
port = 3306
command = "mariadbd"

# Optional: override TUI colors. Values are hex strings (#rrggbb) or named
# terminal colors (red, green, cyan, white, darkgray, lightgreen, …).
# All fields are optional — omit to keep the default.
[theme]
border        = "#3c3c50"  # unfocused pane borders
border_focus  = "#96c832"  # focused pane border (lime green)
text          = "#c8c8d2"  # primary text
text_dim      = "#646478"  # timestamps, labels, key hints
accent        = "#78b4dc"  # project type, port numbers
ok            = "#64c864"  # running status, managed badge
warn          = "#dcb43c"  # warnings, adopted badge, stderr lines
err           = "#dc5a5a"  # errors, failed status
highlight_bg  = "#282837"  # selected-item background
```

---

### Deploy locally
```
cargo install --path . --force
strip ~/.cargo/bin/zapusk
```

---

## TODO / Ideas

### Features
- [ ] Watch config file for changes and hot-reload project list

### Distribution
- [ ] Shell completions (bash, zsh, fish) via `clap`
- [ ] Homebrew formula
