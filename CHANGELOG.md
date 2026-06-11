# Changelog

All notable changes to zapusk are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
