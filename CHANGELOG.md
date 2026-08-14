# Changelog

All notable changes to zapusk are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.17]

### Added

- **Color themes** as TOML files. `[theme] name = "groknight"` (default) or
  `"terminal"` (follow the terminal's own palette). Drop a file in
  `~/.config/zapusk/themes/<id>.toml` to add more — same overlay model as
  framework recipes. Slot overrides on `[theme]` still work. `zapusk doctor`
  lists loaded themes. Example: [`themes.example/tokyonight.toml`](themes.example/tokyonight.toml).
  Press `t` in the TUI to pick a theme from a list (same flow as `l` for
  language). Shipped palettes also include **Nightfox**, **Catppuccin**
  (Mocha), and **Macintosh 1984** light/dark (vintage phosphor + Apple
  16-color status, inverted selection).

### Changed

- Default TUI chrome matches Grok Build: **GrokNight** palette on a padded
  `#141414` canvas, faint square frames, `›` selection, a quiet header
  (`zapusk  |  3 projects  |  …`), a boxed `>` prompt (status or selected
  project, language + version on the right), and a footer of
  `key:label  |  key:label` that follows the open overlay. Unmanaged
  processes stay a header count plus the `u` popup.

## [0.1.16]

### Changed

- `l` opens a language picker panel (list + j/k + Enter) instead of cycling blindly.

## [0.1.15]

### Added

- French and Italian UI (`locales/fr.toml`, `locales/it.toml`). `l` now cycles
  English → Deutsch → Français → Italiano → Srpski → Русский.

## [0.1.14]

### Changed

- UI translations moved from Rust `match` arms to [`locales/*.toml`](locales/).
  Translators can edit those files directly. Missing keys fall back to English.
  `~/.config/zapusk/locales/<code>.toml` overlays the shipped file.

## [0.1.13]

### Added

- **UI languages**: English, German, Serbian (Latin), and Russian. Set
  `language = "de"` (or `sr` / `ru` / `en`) in `config.toml`, or press `l` in
  the TUI to cycle. If unset, zapusk uses `LANG` / `LC_MESSAGES`. The choice is
  saved back to the config.

## [0.1.12]

### Added

- **Config hot-reload**: the TUI polls `config.toml` and updates the project list
  when the file changes (another editor, `zapusk add`, …). Running processes are
  never started or stopped by a reload; removed running projects are forgotten
  and left alive (same as `q`). Command/port/path changes on a running project
  take effect on the next start. Invalid TOML is ignored and reported in the
  status bar. Reloads are deferred while the add/edit/confirm UI is open. TUI
  saves of the same file do not flicker a reload.
- **Shell completions**: `zapusk completions <shell>` prints a clap-generated
  script for bash, zsh, fish, elvish, or powershell.

## [0.1.11]

### Added

- **Framework recipes**: project types are now TOML specs, not a closed Rust enum.
  Built-ins (`phoenix`, `symfony`, `kirby`, `axum`, `compose`) ship inside the
  binary and keep working with existing `config.toml` files.
  Drop a file in `~/.config/zapusk/frameworks/<id>.toml` to add Rails, Laravel,
  Express, or anything else — no recompile. User files with the same `id` override
  a built-in. Example recipes live in `frameworks.example/`.
- `zapusk doctor` lists loaded framework recipes and their source (builtin/user).
- `zapusk init` / `zapusk add` create the user `frameworks/` directory.

### Changed

- `type` in `config.toml` is a free string looked up in the recipe registry.
  Unknown types fail at start/doctor with a pointer to the frameworks directory.
- Caddy site generation, start commands, env vars, doctor checks, discovery
  import, and domain-verify timeouts all come from the recipe (plus a small
  closed set of hooks: PHP version file, PHP binary, compose lifecycle,
  `static_plus_proxy`).

## [0.1.10]

### Fixed

- High idle CPU usage (~13–20%, even with no projects running and no traffic).
  The TUI redrew the entire screen on a fixed ~10×/second cadence whether or not
  anything had changed, and polled the keyboard with a blocking call inside the
  async runtime. The render loop is now event-driven (crossterm `EventStream`
  plus `tokio::select!`): it parks until there is actual terminal input, new log
  output, a process/state change, or an animation frame, and coalesces bursts
  into a single redraw. The spinner now advances only while a project is
  starting, rather than on every frame. Idle CPU drops to roughly 0.5–3%.

## [0.1.9]

### Fixed

- Symfony projects now honor the `php_version` set in `config.toml`. The Symfony
  CLI only picks a PHP version from a `.php-version` file in the project root, so
  zapusk manages that file from its own config (the single source of truth)
  before starting: when `php_version` is set, `.php-version` is written/updated
  to match; when it is unset, an existing `.php-version` is removed so the
  project falls back to the default PHP. This fixes both a project running the
  system default instead of the configured version, and a stale pin lingering
  after `php_version` is removed. Also removed an unsupported `--php-version`
  flag that was being passed to `symfony server:start`.

## [0.1.8]

### Changed

- The project list now groups running projects above stopped ones, each group
  alphabetical by name, with a blank line between the two groups (no group
  titles). This is display-only: `config.toml` keeps its original order, and
  `j`/`k` navigation follows the on-screen order.

## [0.1.7]

### Added

- **Docker Compose project type** (`type = "compose"`): run a project's whole
  stack (app, db, redis, …) in containers — useful for team members without
  native databases, PHP, etc. Works on macOS (Docker Desktop, OrbStack,
  colima) and Linux (docker engine).
  - zapusk runs `docker compose up` in the foreground as the tracked process;
    all services' logs stream into the log pane with `service |` prefixes,
    including image pull progress and container create/start steps on first
    start.
  - Stop runs `docker compose stop -t 10` so containers shut down gracefully
    and are never orphaned.
  - Soft quit (`q`) leaves the stack running; the next launch re-adopts it.
    Stacks started externally (`docker compose up -d`) are detected and
    adopted with logs re-attached.
  - New optional config fields: `compose_file` (default: auto-detect
    `compose.yaml` / `compose.yml` / `docker-compose.yml` /
    `docker-compose.yaml`), `service`, `compose_profiles`.
  - `PORT` is exported to the compose process, so compose files can publish
    the configured port with `ports: ["${PORT}:80"]`.
  - Caddy proxying, `tls`, and `aliases` work unchanged for compose projects.
- `zapusk doctor`: docker daemon and compose-v2-plugin checks with per-OS fix
  hints (only run when compose projects exist in the config; warns when the
  EOL docker-compose v1 binary is the only one found, hints at podman-docker
  when only podman is installed).
- `zapusk init`: new step 6/6 checks Docker when compose projects are
  configured (skipped otherwise).
- `zapusk add`: offers the `compose` type and prompts for the compose file
  and main service.
- First unit tests (config parsing, compose file resolution, compose command
  construction).

### Fixed

- `zapusk --version` / `-V` now works (the version flag was never enabled in
  the CLI definition).

## [0.1.6] and earlier

Pre-changelog releases — see the git history. Highlights: Phoenix / Symfony /
Kirby / Axum project types, Caddy reverse proxy with `tls internal`, dnsmasq
wildcard DNS, process adoption via pidfiles and port discovery, unmanaged
service discovery, inline add/edit forms, themes, `doctor` / `init` / `add` /
`destroy` / `discover` subcommands.
