# Roadmap

Ideas for zapusk. Not a promise or a schedule. Rafael decides what gets
built; suggest changes via [GitHub issues](https://github.com/e9li/zapusk/issues).
See [CONTRIBUTING.md](CONTRIBUTING.md).

The product stays a **local** multi-project TUI: `.test` domains, Caddy,
start/stop/adopt, TOML recipes. The next gains are operations, not more chrome.

---

## Do next

### Filter the project list

Typeahead or `/`-style filter on name, type, and domain. Running/stopped
grouping stays. Logs already have `/`; the list does not.

### Groups / sessions

Named sets of projects, e.g. `work = [site, api, redis]`, with start/stop
all. Matches a typical day better than another theme.

### Crash and health after start

Start already verifies the domain once. If a process dies later, the TUI
should show **failed** and optionally restart (`restart = "on-crash"` per
project). Not a file-watcher that restarts on every save.

### TUI forms vs `config.toml`

Add/edit should cover the fields people actually set, or say what is
config-only. Today TOML has `php_version`, `autostart`, `env`,
`command`/`args`, and compose fields that the forms omit.

### Recipe registry live-reload

Config hot-reload is done. Dropping `~/.config/zapusk/frameworks/rails.toml`
should show up in `a` without restarting the TUI.

---

## Distribution

Plan: [docs/homebrew.md](docs/homebrew.md).

- [ ] Tag a release that matches `Cargo.toml`
- [ ] GitHub release / public tarball
- [ ] `e9li/homebrew-tap` and `brew install e9li/tap/zapusk`
- [ ] README install one-liner
- [ ] (Later) bottles
- [ ] (Much later) `homebrew/core`

---

## Nice, not urgent

| Idea | Notes |
|------|--------|
| Trust Caddy `tls internal` on macOS | One `doctor` / `init` step so Safari/curl stop complaining |
| Copy last error / export logs | Debugging already lives in the log pane |
| `zapusk recipe init` | Scaffold a TOML; today copy `frameworks.example/` |
| Linux packages | Only if install outside Homebrew/cargo matters |
| More shipped recipes | Users can add files; do not grow the binary for fashion |

---

## Not planned

These would dilute the product or fight the current design:

- PHP-FPM / production-shaped PHP (local is `php -S` and Symfony CLI)
- Restart on every file change (noisy; fights Phoenix/Vite)
- Gettext / CLI translations (TUI languages are enough)
- Multi-user, remote, or SSH control
- Accepting pull requests on GitHub (issues only)
- WASM / `.so` framework plugins (TOML recipes + closed capabilities)
- Replacing Docker Compose or ddev
