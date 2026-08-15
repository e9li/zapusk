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

Built-in recipes (shipped in the binary):

| Type | How it runs |
|------|-------------|
| **Phoenix** | `mix phx.server` |
| **Symfony** | `symfony server:start` (PHP version from `php_version`, written to `.php-version`) |
| **Kirby** | PHP built-in server (`php -S`), proxied by Caddy (Homebrew PHP, version per project) |
| **Axum** | `cargo run` |
| **Compose** | `docker compose up` (foreground) — the whole stack (app, db, redis, …) runs in containers |

Add more types without recompiling: drop a TOML recipe in `~/.config/zapusk/frameworks/`. Ready-made examples for **Rails**, **Laravel**, and **Express** are in [`frameworks.example/`](frameworks.example/).

```bash
mkdir -p ~/.config/zapusk/frameworks
cp frameworks.example/rails.toml ~/.config/zapusk/frameworks/
# then in config.toml:  type = "rails"
```

The design is intentionally stack-agnostic — anything that binds to a port can be a recipe. Caddy proxies it; `zapusk` manages the process.

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

That said, Docker is supported *per project* via `type = "compose"` — useful when team members (e.g. on Linux) don't have native databases or PHP installed. Caddy and dnsmasq stay native; only the project's services run in containers. See [Compose projects](#compose-projects-docker) below.

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

### Symfony: PHP version

The Symfony CLI selects the project's PHP version from a `.php-version` file in
the project root — it has no command-line flag for it. zapusk treats its own
config as the single source of truth and manages that file for you:

- Set `php_version` on a Symfony project → zapusk writes (or updates)
  `.php-version` to match before starting, so the configured version is used
  instead of the system default.
- Remove `php_version` from the config → zapusk deletes `.php-version` on the
  next start, so the project falls back to the default PHP.

Because zapusk owns this file, configure the PHP version in `config.toml`, not
by hand-editing `.php-version` (your edits there are overwritten or removed to
match the config).

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

### Compose projects (Docker)

A project with `type = "compose"` is started as `docker compose up` (foreground, with `--no-color --remove-orphans`) in the project directory. The compose CLI is the tracked process: all services' logs stream into the log pane with `service |` prefixes, and stopping the project runs `docker compose stop -t 10` so containers shut down gracefully.

On first start, image pull progress and container create/start steps are visible in the log pane (compose prints plain line-by-line progress when writing to log files), so long pulls don't look like a hang.

The compose file belongs to the project repo — zapusk never generates or edits it. It is resolved from `compose_file` in the config, or auto-detected (`compose.yaml`, `compose.yml`, `docker-compose.yml`, `docker-compose.yaml`).

zapusk exports `PORT` to the compose process, so the recommended convention is to publish the web service's port with interpolation — then the published port always matches the zapusk config:

```yaml
# compose.yaml (in the project repo)
services:
  web:
    build: .
    ports:
      - "${PORT:-8080}:80"
  db:
    image: postgres:17
    volumes:
      - db-data:/var/lib/postgresql/data
volumes:
  db-data:
```

Caddy proxies `name.test` → `localhost:<port>` exactly like for native projects; `tls` and `aliases` work unchanged.

Notes:

- Works with Docker Desktop, OrbStack, and colima on macOS, and docker engine on Linux — anything that provides the `docker` CLI.
- Soft quit (`q`) leaves the stack running; on the next launch zapusk re-adopts it. A stack started externally (`docker compose up -d`) is detected and adopted too (logs re-attached via `docker compose logs -f`).
- `zapusk doctor` checks the docker daemon and compose plugin, but only when compose projects exist in the config.
- Cold starts (image pulls, db init) get an extended domain-verification window (~100s instead of ~20s).

---

## Architecture

```
~/.config/zapusk/
├── config.toml       ← project registry + caddy settings
├── frameworks/       ← user recipes (rails.toml, express.toml, …); override builtins by id
├── Caddyfile         ← auto-generated, do not edit manually
├── logs/             ← stdout/stderr log files per project (<name>.out, <name>.err)
└── pids/             ← pidfiles for process re-adoption across sessions (<name>.pid)
```

```
locales/              # UI translations: en, de, fr, it, sr, ru
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
│   ├── config.rs     # TOML config deserialization + FrameworkId
│   ├── framework.rs  # Recipe registry (builtins + ~/.config/zapusk/frameworks)
│   ├── docker.rs     # docker compose CLI detection + up/ps/logs/stop commands
│   └── discovery.rs  # Listening-port discovery + recipe heuristics
├── frameworks/       # Shipped builtin recipes (phoenix, symfony, kirby, axum, compose)
├── i18n.rs           # Language switch + TOML catalog loader
└── cli/
    ├── doctor.rs     # `zapusk doctor` — dependency checks
    ├── init.rs       # `zapusk init` — interactive first-run setup
    ├── add.rs        # `zapusk add` — add project interactively
    ├── destroy.rs    # `zapusk destroy` — remove all zapusk configuration
    ├── discover.rs   # `zapusk discover` — list unmanaged listening apps
    └── completions.rs # `zapusk completions` — clap shell scripts
```

---

## CLI subcommands

`zapusk` is invoked without arguments to open the TUI. It also supports subcommands:

```
zapusk              # open TUI
zapusk init         # interactive first-run setup
zapusk doctor       # check all dependencies
zapusk add          # add a project to config interactively
zapusk start NAME   # start a project (same manager as the TUI)
zapusk stop NAME
zapusk restart NAME
zapusk status       # list all (alias: list); status NAME for one
zapusk open NAME    # open http(s)://domain in the browser
zapusk destroy      # remove all zapusk configuration
zapusk discover     # discover listening services (managed + unmanaged)
zapusk discover --import 4000  # import discovered service by port/pid
zapusk completions zsh   # print a completion script (also bash/fish/elvish/powershell)
```

`start` / `restart` rewrite the Caddyfile when `[caddy]` is set, then wait
until the domain answers (`--no-wait` skips that). `start` on an already
running project exits 0. Processes keep running after the command exits
(same pidfiles as the TUI). `status --json` is for scripts.

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
- **PHP:** per-version binary present (only if a recipe sets `require_php` / `resolve_php_binary`)
- **Docker:** daemon reachable, compose v2 plugin present (only if compose projects exist)
- **Frameworks:** loaded recipes (builtin vs user) and parse errors
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

[1/6] Checking Caddy...
      ✓ caddy found (2.8.4)

[2/6] Checking dnsmasq...
      ✗ dnsmasq not found
      → Install dnsmasq? [Y/n]
        macOS:  brew install dnsmasq
        Linux:  sudo apt install dnsmasq
      Running: brew install dnsmasq ... done

[3/6] Configuring dnsmasq for *.test...
      ✓ address=/.test/127.0.0.1 already present

[4/6] Starting dnsmasq...
      → Start dnsmasq now? [Y/n]
        Running: brew services start dnsmasq ... done

[5/6] Generating Caddyfile from config...
      → Config found at ~/.config/zapusk/config.toml
      ✓ Caddyfile written to ~/.config/zapusk/Caddyfile
      → Reload Caddy? [Y/n]  done

[6/6] Checking Docker (compose projects)...
      ✓ Skipped — no compose projects in config

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
| `l` | Open the language picker (English, Deutsch, Français, Italiano, Srpski, Русский) |
| `t` | Open the theme picker (shipped palettes plus `~/.config/zapusk/themes/`) |
| `q` | Quit (keeps running projects alive) |
| `Q` | Force quit (stops projects, then tries to stop Caddy/dnsmasq) |

Inside the unmanaged services popup (`u`): `j/k` select, `Enter` inspect,
`i` import as project, `I` ignore, `f` toggle stack filter (`dev-only`/`all`),
`w` toggle port filter (`web`/`all-ports`), `r` refresh, `Esc` close.

Layout: a quiet header (`zapusk | N projects | Caddy/dnsmasq`), projects +
details on the left, logs on the right, a `>` prompt row, and a footer of
`key:label  |  key:label` shortcuts. Unmanaged listeners show as a header
count and open with `u`.

Project list badges: `[M]` = managed by zapusk, `[A]` = adopted external process.

Project list also shows `tls:on` / `tls:off` per project.

The project list groups **running projects above stopped ones**, alphabetically
by name within each group, with a blank line between the two groups. This is a
display-only ordering — your `config.toml` keeps its original order, and a
project moves between groups as you start/stop it (the cursor follows it).

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

## Framework recipes

A recipe is a TOML file with an `id` (used as `type` in `config.toml`). Built-ins live inside the binary; user files in `~/.config/zapusk/frameworks/*.toml` are merged on top (same `id` overrides the builtin).

Minimal example (`~/.config/zapusk/frameworks/rails.toml`):

```toml
id = "rails"

[start]
command = "bin/rails"
args = ["server", "-p", "{port}", "-b", "127.0.0.1"]

[doctor]
binaries = ["ruby"]
marker_files = ["Gemfile", "config.ru"]
```

### Placeholders

Substituted in `start.command`, `start.args`, `env` values, and Caddy `root` / `block_paths`:

| Token | Source |
|-------|--------|
| `{port}` | project `port` |
| `{path}` | project `path` |
| `{domain}` | project `domain` |
| `{name}` | project `name` |
| `{php_version}` | project `php_version` (empty if unset) |
| `{public_dir}` | project `public_dir`, default `public` |
| `{root}` | `{path}/{public_dir}`, or `{path}` when `public_dir = "/"` |
| `{php}` | resolved PHP binary when `hooks.resolve_php_binary` is set, else `php` |

Per-project `command` / `args` still override the recipe start command. Per-project `env` is merged on top of the recipe env. `PORT` is always exported.

### Optional sections

```toml
[env]
PHX_HOST = "{domain}"

[lifecycle]
kind = "native"          # native | compose
ready_attempts = 8       # domain-verify retries (compose builtin uses 40)

[caddy]
profile = "proxy"        # proxy | static_plus_proxy
root = "{root}"          # static_plus_proxy only
block_paths = ["/.*"]    # static_plus_proxy only

[hooks]
sync_php_version = false     # write/delete .php-version from php_version
resolve_php_binary = false   # resolve {php} via Homebrew php@X.Y
require_php = false          # doctor checks php@version is installed

[discovery]
command_contains = ["puma"]
cwd_contains = ["config/application.rb"]
```

Unknown hook names and unknown Caddy profiles are rejected when the file is loaded (`zapusk doctor` lists the error). Recipes cannot embed raw Caddy snippets or shell scripts.

Copy the samples:

```bash
cp frameworks.example/rails.toml ~/.config/zapusk/frameworks/
cp frameworks.example/laravel.toml ~/.config/zapusk/frameworks/
cp frameworks.example/express.toml ~/.config/zapusk/frameworks/
```

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
php_version = "8.1"        # kirby: selects the Homebrew PHP binary directly
path = "/home/user/projects/company-site"
# public_dir = "public"    # optional: document root subfolder (default: "public")

[[projects]]
name = "blog"
domain = "blog.test"
port = 8002
type = "symfony"
php_version = "8.3"        # optional: synced to .php-version for the Symfony CLI
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

[[projects]]
name = "shop"
domain = "shop.test"
port = 8080                # host port published by the compose stack
type = "compose"           # runs `docker compose up` in the project dir
path = "/home/user/projects/shop"
tls = true
# compose_file = "docker-compose.dev.yml"  # optional, default: auto-detect
# service = "web"                          # optional: main service name
# compose_profiles = ["dev"]               # optional: --profile flags

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

# Color theme. `name` picks a palette; other keys overlay it.
[theme]
name          = "groknight"  # groknight | terminal | nightfox | catppuccin | macintosh | macintosh-dark | user id
bg            = "#141414"  # application canvas
border        = "#414141"  # unfocused pane borders
border_focus  = "#bb9af7"  # focused pane
text          = "#e1e1e1"  # primary text
text_dim      = "#6c6c6c"  # timestamps, labels, key hints
accent        = "#bb9af7"  # titles, keys, project type
ok            = "#9ece6a"  # running status, managed badge
warn          = "#e0af68"  # warnings, adopted badge, stderr
err           = "#f7768e"  # errors, failed status
highlight_bg  = "#242424"  # selected-item background
highlight_fg  = "#e1e1e1"  # selected-item text (omit to use `text`)
```

---

## Themes

Shipped palettes:

| Name | What it does |
|------|----------------|
| **groknight** | Default. Near-black canvas, magenta accent (Grok Build). |
| **terminal** | Uses the terminal's own background/foreground and 16 ANSI colors, so zapusk follows iTerm, Ghostty, Alacritty, … |
| **nightfox** | EdenEast Nightfox — deep blue-gray canvas, purple accent. |
| **catppuccin** | Catppuccin Mocha — pastel mauve accent on a warm dark base. |
| **macintosh** | Macintosh 1984 light — classic computer beige, 1-bit chrome, Apple 16-color status. |
| **macintosh-dark** | Same beige in low light: dark brown chassis, warm ivory type. |

```toml
[theme]
name = "terminal"
```

Add more without recompiling: drop a TOML file in `~/.config/zapusk/themes/`. Same `id` as a builtin replaces it. An example (Tokyo Night) is in [`themes.example/`](themes.example/).

```toml
# ~/.config/zapusk/themes/tokyonight.toml
id = "tokyonight"
bg = "#1a1b26"
text = "#c0caf5"
accent = "#bb9af7"
# … see themes.example/tokyonight.toml
```

```toml
[theme]
name = "tokyonight"
accent = "#7aa2f7"   # optional override
```

Color values are `#rrggbb`, named ANSI colors, or `reset` (terminal default). Config hot-reload applies theme changes.

---

## Languages

The TUI is available in **English**, **German**, **French**, **Italian**, **Serbian** (Latin), and **Russian**.
Strings live in [`locales/`](locales/) (`en.toml`, `de.toml`, `fr.toml`, `it.toml`, `sr.toml`, `ru.toml`) so a new
language is another TOML file, not a Rust change.

```toml
# ~/.config/zapusk/config.toml
language = "de"    # en | de | fr | it | sr | ru
```

If `language` is unset, zapusk follows `LANG` / `LC_MESSAGES` (`de_*`, `fr_*`, `it_*`, `sr_*`, `ru_*`).
In the TUI, press `l` to open the language list, then `j`/`k` and Enter to choose.
The choice is written back to `config.toml`. Press `t` for the same flow with color themes.

To preview a translation without rebuilding, copy a file to
`~/.config/zapusk/locales/de.toml` — it overlays the shipped file (same keys).
Keep `{name}` / `{port}` / … placeholders unchanged.

CLI subcommands (`doctor`, `init`, …) stay in English.

## Config hot-reload

While the TUI is open, zapusk polls `~/.config/zapusk/config.toml` about twice a
second. Saving the file in an editor updates the project list:

- New projects appear as **stopped** (`autostart` is launch-only, not applied on reload)
- Removed projects leave the list. If they were running, the process is **not**
  killed (same contract as `q`); zapusk just stops tracking it
- Field edits apply immediately for the next start. A running project whose
  command, port, or path changed is **not** restarted — the status bar asks you to
- Domain / TLS / alias changes regenerate the Caddyfile
- Invalid TOML keeps the current list and shows the parse error
- Reloads wait if the add/edit form or a confirm dialog is open
- Adding a **framework recipe** still needs a TUI restart (the recipe registry
  is loaded at startup)

## Shell completions

```bash
# zsh
mkdir -p ~/.zfunc
zapusk completions zsh > ~/.zfunc/_zapusk
# in ~/.zshrc:  fpath=(~/.zfunc $fpath) && autoload -Uz compinit && compinit

# bash
mkdir -p ~/.local/share/bash-completion/completions
zapusk completions bash > ~/.local/share/bash-completion/completions/zapusk

# fish
zapusk completions fish > ~/.config/fish/completions/zapusk.fish
```

`zapusk completions` also accepts `elvish` and `powershell`.

## License

Created and maintained by **Rafael Egli**. Copyright (c) 2026 **e9li GmbH**,
Switzerland. Released under the [MIT License](LICENSE.md) (stated 2026-08-15):
use it freely; it comes **as is**, without warranty. Rafael Egli and e9li GmbH
are not responsible for problems caused by using this software. Tagged
releases keep the license file they shipped with.

## Contributing

Please **open an issue** on the GitHub mirror:
<https://github.com/e9li/zapusk/issues>.

Pull requests are not accepted. The GitHub repo is for issues and browsing;
the canonical source is <https://git.e9li.com/e9li/zapusk>. See
[CONTRIBUTING.md](CONTRIBUTING.md).

## Roadmap

See [ROADMAP.md](ROADMAP.md). Homebrew plan: [docs/homebrew.md](docs/homebrew.md).
