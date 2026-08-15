# Homebrew plan

How to ship zapusk with Homebrew. This is a **personal/org tap** first.
`brew install zapusk` from `homebrew/core` is a later step.

The formula does **not** live in this repo. This repo stays the app;
packaging lives in a separate tap (`e9li/homebrew-tap` on GitHub).

---

## Status

- [x] MIT license ([LICENSE.md](../LICENSE.md))
- [ ] Tag a release that matches `Cargo.toml` (currently `0.1.18`; existing
      tags are only `v0.1.0` / `v0.1.1` / `v0.1.8`)
- [ ] Public tarball URL for that tag
- [ ] Create `e9li/homebrew-tap`
- [ ] Add `Formula/zapusk.rb`
- [ ] `brew install --build-from-source`, `brew test`, `brew audit`
- [ ] README install one-liner
- [ ] Repeatable bump process for later versions
- [ ] (Later) bottles via the tap’s GitHub Actions
- [ ] (Much later) PR to `homebrew/core`

---

## Before the formula

1. **Tag a real release** on the canonical repo and the GitHub mirror,
   matching `Cargo.toml` (e.g. `v0.1.18`).

   - Canonical: <https://git.e9li.com/e9li/zapusk>
   - Mirror (issues): <https://github.com/e9li/zapusk>

2. **Public HTTP tarball.** Homebrew downloads source without git login.
   Prefer the GitHub archive:

   ```
   https://github.com/e9li/zapusk/archive/refs/tags/v0.1.18.tar.gz
   ```

   `git.e9li.com` only works if that archive URL is public without auth.

3. **License** is already MIT. The formula should say `license "MIT"`.

Do **not** make Caddy or dnsmasq hard dependencies. The binary runs
without them; `zapusk init` / `doctor` install and check the stack.

---

## One-time: create the tap

A tap is a **separate** GitHub repo. Homebrew maps `e9li/tap` →
`e9li/homebrew-tap`.

```bash
brew tap-new e9li/homebrew-tap
cd "$(brew --repository e9li/homebrew-tap)"
gh repo create e9li/homebrew-tap --public --source . --remote origin --push
```

`brew tap-new` also adds GitHub Actions that can build **bottles**
(prebuilt binaries) so users do not need a Rust toolchain.

---

## Add the formula

After the tag exists:

```bash
brew create --rust \
  https://github.com/e9li/zapusk/archive/refs/tags/v0.1.18.tar.gz \
  --tap e9li/homebrew-tap \
  --set-name zapusk
```

That opens `Formula/zapusk.rb`. Target shape:

```ruby
class Zapusk < Formula
  desc "TUI for managing local web development projects"
  homepage "https://git.e9li.com/e9li/zapusk"
  url "https://github.com/e9li/zapusk/archive/refs/tags/v0.1.18.tar.gz"
  sha256 "PASTE_SHA256"
  license "MIT"
  head "https://git.e9li.com/e9li/zapusk.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args
    generate_completions_from_executable(bin/"zapusk", "completions")
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/zapusk --version")
  end
end
```

SHA of the tarball:

```bash
curl -fsSL https://github.com/e9li/zapusk/archive/refs/tags/v0.1.18.tar.gz | shasum -a 256
```

---

## Check it locally

```bash
HOMEBREW_NO_INSTALL_FROM_API=1 brew install --build-from-source e9li/tap/zapusk
brew test zapusk
brew audit --strict --online zapusk
brew style --fix --formula zapusk
```

Commit `Formula/zapusk.rb` and push the tap.

---

## What users run

```bash
brew install e9li/tap/zapusk
```

or:

```bash
brew tap e9li/tap
brew install zapusk
```

Add this one-liner to the zapusk README when the tap is live.

---

## Each new version

1. Finish work here, bump `Cargo.toml` and `CHANGELOG.md`.
2. Tag `vX.Y.Z` on git.e9li.com **and** GitHub.
3. In the **tap** repo, update `url` and `sha256` (Homebrew infers the
   version from the URL).
4. Run `brew audit --strict --online zapusk` again.
5. Commit and push the tap.

If the tap’s bottle workflow is enabled, CI builds bottles and later
`brew install` skips compiling Rust.

---

## `homebrew/core` later

That is a PR to Homebrew’s main formulae repo. Same formula shape, much
stricter review (stable, used, documented). A working tap is the usual
stepping stone. Do not start that until the tap has been used in the wild.
