# Agent guide

Conventions for working in this repo. User-facing docs live in [README.md](README.md), developer setup in
[CONTRIBUTING.md](CONTRIBUTING.md).

## Layout

| Path             | What                                                                         |
| ---------------- | ---------------------------------------------------------------------------- |
| `crates/server`  | gRPC server (`server` binary) that talks to Jackett and Deluge               |
| `crates/client`  | `client` binary: ratatui TUI, plus `client certs` for generating mTLS certs  |
| `crates/shared`  | Generated gRPC code (`shared::api`) and mTLS cert handling (`shared::certs`) |
| `proto/<svc>/v1` | Protobuf definitions, one directory per service                              |
| `hack/certs`     | Dev certs from `mise run certs:dev` (gitignored)                             |

## Commands

All tooling (Rust, `protoc`, `cargo-shear`, `actionlint`, `shellcheck`) comes from mise. Run tasks through mise rather
than calling the tools directly, so local runs match CI.

```sh
mise run test:ci      # everything CI runs: fmt check, clippy, unused deps, workflow lint, tests, build
mise run fmt          # format (nightly rustfmt)
mise run test         # tests only
```

`mise run test:ci` must pass before committing.

## Rust style

- **Formatting:** run `mise run fmt` regularly, and always before committing. `rustfmt.toml` is the source of truth;
  don't hand-format around it.
- **Lints:** clippy runs with `-D warnings`. Never use `mod.rs`; a module with children is `foo.rs` plus a `foo/`
  directory.
- **Dependencies:** declare versions once in the root `[workspace.dependencies]` and use `dep.workspace = true` in
  crates. `cargo shear` fails CI on unused deps. If a dep is only used inside macro-generated code, add it to that
  crate's `[package.metadata.cargo-shear] ignored` list with a comment saying why.
- **Errors:** library modules (`jackett`, `deluge`) define their own `pub enum Error` with hand-written `Display` and
  `From` impls (no `thiserror`/`anyhow`). Binaries and the client use `color_eyre::Result`.
- **gRPC errors:** services map library errors to `tonic::Status` in a `to_status` function. Status messages must never
  contain secrets (API keys, passwords); tests assert this.
- **TLS:** rustls with the `ring` provider. The server binary installs it as the process default at startup, because
  reqwest uses the process-wide provider. Tests that build Jackett or Deluge clients do the same with
  `let _ = rustls::crypto::ring::default_provider().install_default();`.
- **Doc comments:** document public functions and types with `///`, especially anything non-obvious (defaults, side
  effects, error cases).

## Configuration and secrets

- Secrets come from env vars named `WATCHIT_*` (clap `env = "..."` with `hide_env_values = true`), never from flags
  documented with real values.
- `.envrc` holds real local secrets and is gitignored. Never print it, commit it, or copy values from it.
- Test data must be synthetic: `localhost` URLs, placeholder keys like `secret`, fake hashes. Never capture fixtures
  from a real Jackett or Deluge.
- Generated keys are written `0600` and existing cert files are never overwritten (`shared::certs::Bundle::save`).

## Protobuf

- Each service lives at `proto/<service>/v1/<service>.proto` with `package <service>.v1;`.
- `crates/shared/build.rs` compiles each file explicitly. A new proto needs a `compile_protos` line there and a module
  in `crates/shared/src/api.rs` that includes it and re-exports the client/server types.

## Testing

- **Test the public API by default.** Tests go in `crates/<crate>/tests/<module>.rs` and import from the crate like any
  other user would.
- **Unit tests only when the public API makes it awkward**, for example TUI internals or a pure mapping function that
  would need many mock setups to reach. Put them in `src/<module>/tests.rs` and declare `#[cfg(test)] mod tests;` at the
  top of the module, above the `use` blocks. Don't make items `pub` just to test them.
- Mock external HTTP services (Jackett, Deluge) with `wiremock`. Shared fixtures live in `crates/<crate>/testdata`.
- When moving or restructuring tests, compare `cargo test --workspace -- --list` before and after to prove none were
  lost.

## Commits

- [Conventional Commits](https://www.conventionalcommits.org): `feat`, `fix`, `refactor`, `build`, `ci`, `docs`,
  `chore`. Lowercase, imperative subject, no trailing period.
- The body explains why, not just what.
- One concern per commit. Stage files by name; never `git add .` or `git add -A`.

## Versioning and releases

- The version lives only in the root `[workspace.package]`. Don't bump it by hand; the Release workflow does it.
- Releasing: run the **Release** workflow on `main` with `patch`, `minor` or `major`. It bumps `Cargo.toml` and
  `Cargo.lock`, commits `chore: release vX.Y.Z` to `main`, tags it, builds every target in `rust-toolchain.toml`
  natively, and publishes `watchit-{server,client}-vX.Y.Z-<target>.tar.gz` plus `SHA256SUMS`.
- If a build fails after the tag is pushed, use **Re-run failed jobs**. Dispatching again bumps the version a second
  time.

## GitHub Actions

- Pin every action to a full commit SHA with the version in a trailing comment:
  `uses: actions/checkout@<40-char sha> # v7.0.1`. Resolve annotated tags to the commit they point at.
- Before adding or updating an action, look up its current release (`gh api repos/<owner>/<repo>/releases`) instead of
  relying on memory.
- Dependabot (`.github/dependabot.yaml`) bumps action SHAs and cargo deps weekly.
- Workflows are linted by `mise run lint:actions` (actionlint + shellcheck). Pass inputs and step outputs to `run:`
  scripts through `env:`, not by interpolating `${{ }}` into the script.

## Tooling

- Add new tools to `[tools]` in `mise.toml`, then run `mise lock` so `mise.lock` has checksums for every platform CI
  uses.

## Docs and prose

Applies to docs, comments, commit messages and PR text.

- Sentence-case headings.
- No emojis. No em dashes or en dashes; use commas, semicolons or a plain hyphen.
- Keep README.md for people using watchit and CONTRIBUTING.md for people working on it.

## Gotchas

- **rust-analyzer rewrites `Cargo.lock`** whenever `Cargo.toml` changes on disk. Avoid temporary edits to `Cargo.toml`,
  check `git diff Cargo.lock` right before staging, and run `cargo build --locked` after committing manifest changes.
- **zsh reserves `path`** (it's tied to `$PATH`). Don't use it as a variable name in shell scripts.
- `cargo test` alone doesn't catch everything CI does; use `mise run test:ci`.
