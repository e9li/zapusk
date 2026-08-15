# Plan: CLI without the TUI

First item under [ROADMAP.md](../ROADMAP.md) “Do next”. The TUI stays the
main UI. These commands call the **same** `Manager` so a project started in
the TUI can be stopped from the shell (and the reverse), via pidfiles.

---

## Goal

```
zapusk start <name>
zapusk stop <name>
zapusk restart <name>
zapusk status              # all projects
zapusk status <name>       # one project
zapusk list                # alias of status
zapusk open <name>
```

Use from scripts, Raycast, or `cd ~/proj && zapusk start myshop`.

**Non-goals (this slice)**

- Start/stop groups (later roadmap item)
- Filter / typeahead in the TUI
- i18n for CLI (doctor/add stay English)
- Confirm prompts (CLI is non-interactive)
- File-watch restart
- `--all` (can add later; not required to ship)

---

## How it fits today

| Piece | Already exists | CLI must |
|-------|----------------|----------|
| Spawn / adopt / pidfile / compose stop | `Manager::start`, `stop`, `detect_running` | Construct a `Manager`, do **not** duplicate spawn |
| Survive after the zapusk process exits | Child `process_group(0)` + log files | Exit after start; do not kill children |
| Caddyfile + reload | `caddy::write_and_reload` (TUI `ensure_caddy`) | Call on `start` / `restart` when `[caddy]` is set |
| Domain probe | `verify_project_domain_static` in `tui/app.rs` (private) | **Move** to `core` (e.g. `core/ready.rs`) and reuse |
| Browser | `platform::open_url` | Same scheme/domain as `o` in the TUI |
| Completions | `clap_complete` from `Cli` | New subcommands appear automatically |

A **new** CLI process has an empty `Manager` map. `stop` only works if we
**detect/adopt first** (pidfile → compose ps → lsof), then `stop`. Same as
opening the TUI after `q`.

`status` / `list` must **not** call `detect_running`: that starts log-tail
tasks. Add a **read-only probe** (pidfile + `kill(pid,0)`, compose `ps`,
port bind) with no tails and no adopt.

---

## Commands

### Shared

- Load `Config`; fail if missing/invalid (same as other CLI commands).
- Resolve `<name>` to exactly one `[[projects]]` entry, **case-sensitive**
  on `name`. Unknown name → error, list known names, exit `1`.
- No TUI, no i18n. English stdout/stderr.

### `start <name>`

1. Find project.
2. `detect_running`. If already up → print `already running (pid N)` and
   exit `0` (script-friendly; TUI still errors on double-`s`).
3. `write_and_reload` if `[caddy]` is present. Caddy failure is a warning
   on stderr; still try to start (same as TUI).
4. `manager.start`.
5. Wait for domain (default) using the extracted verify helper and
   `lifecycle.ready_attempts`. `--no-wait` skips this.
6. Print `started <name>  http(s)://domain  pid N` (and log path).
7. Exit. Child stays up.

### `stop <name>`

1. Find project.
2. `detect_running`. If not running → `not running`, exit `1`.
3. `manager.stop` (native SIGTERM / compose stop). No `y/n`.
4. Print `stopped <name>`.

### `restart <name>`

`stop` if running (ignore “not running”), sleep 500ms (same as TUI), then
`start`. `--no-wait` forwarded to start.

### `status` / `list`

- No args: one line per project, stable config order.
- With name: that project only; unknown → exit `1`.
- Columns (human): `name  status  pid  type  port  domain`
- `status` = `stopped` | `running` | `adopted` (pidfile/port but not our
  spawn in this process — for CLI, pidfile + alive ≈ running/managed).
  Keep it simple: `running` if probe says up, `stopped` otherwise.
  Optional extra: `via pidfile` / `port` in a last column, or `--json`.
- `--json`: array of objects (`name`, `status`, `pid`, `type`, `port`,
  `domain`, `tls`).

`list` is an alias (`#[command(visible_alias = "list")]` on `status`, or
two clap variants calling the same fn).

### `open <name>`

`http` or `https` from `tls`, then `platform::open_url`. Fail if open
fails. Does not start the project (TUI `o` doesn’t either).

---

## Code layout

```
src/cli/lifecycle.rs   # start / stop / restart / status / open
src/core/ready.rs      # move verify_project_domain_static out of tui/app.rs
src/core/status.rs     # read-only probe (no Manager tails)
src/main.rs            # clap subcommands
```

Keep `cli/*.rs` as thin `run(...)`: load config, resolve name, print,
set exit via `Result` (`main` already returns `Result`).

**Clap** (`src/main.rs`):

```text
zapusk start   <name> [--no-wait]
zapusk stop    <name>
zapusk restart <name> [--no-wait]
zapusk status  [name] [--json]
zapusk list    [name] [--json]    # alias
zapusk open    <name>
```

---

## Extract / small refactors

1. **`verify_project_domain`** → `core/ready.rs` (or `core/caddy.rs` if you
   want fewer files). `tui/app.rs` calls it; CLI `start` calls it.
2. **`probe_running(config, frameworks) -> Option<u32>`** next to pidfile
   helpers in `manager.rs` or `core/status.rs`. Logic copied from
   `detect_running` **without** insert/tail/events.
3. Optional: `fn lookup_project(config, name) -> Result<&ProjectConfig>`
   used by all five commands.

Do **not** pull `App` into the CLI.

---

## Exit codes

| Situation | Code |
|-----------|------|
| Success, including start-when-already-running | 0 |
| Config missing / invalid | 1 |
| Unknown project name | 1 |
| `stop` when not running | 1 |
| `start` spawn or verify failure (unless `--no-wait`; verify fail → 1 after process started) | 1 |
| `open` browser error | 1 |

If start succeeds but domain verify fails, process is left running (same
as TUI). Print the curl error; exit `1` so scripts don’t assume ready.

---

## Tests

- Clap: `Cli::try_parse_from` for each subcommand and `--json` / `--no-wait`.
- `lookup_project`: hit, miss, empty list.
- `probe_running`: pidfile + fake dead pid → none; no network in unit tests.
- Ready helper: keep existing timing/tls `-k` behavior; unit-test URL
  construction if extracted.

No need to spawn real `mix`/`php` in CI for this slice.

---

## Docs

- README: CLI subcommands table + short examples.
- Completions: no extra work (clap).
- CHANGELOG: Added section for the new commands.

---

## Implementation order

1. Extract `verify_project_domain` + `lookup_project` + read-only probe.
2. `status` / `list` (no side effects; easy to test).
3. `open`.
4. `stop` (detect then stop).
5. `start` (caddy + start + wait).
6. `restart`.
7. README + CHANGELOG.

Each step should `cargo fmt` and `cargo test`.

---

## Risks

- **Two zapusk processes:** TUI open + `zapusk stop` from another terminal
  is intended. The TUI already polls/adopts via pidfiles; after stop the
  next TUI tick should show stopped (`check_exited` / port free). Worth a
  manual check.
- **Compose:** `stop` must use compose stop, not SIGKILL — already in
  `Manager::stop` after adopt.
- **Caddy:** CLI start rewriting the Caddyfile while the TUI is open is
  the same as TUI `s` / hot-reload; acceptable.
