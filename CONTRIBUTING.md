# Contributing

## Layout

| Crate    | Binaries | Purpose                                                                          |
| -------- | -------- | -------------------------------------------------------------------------------- |
| `server` | `server` | gRPC server                                                                      |
| `client` | `client` | TUI for searching and watching downloads, plus `certs` for generating mTLS certs |
| `shared` | -        | Generated protobuf/gRPC code from `proto/`, and mTLS cert generation/loading     |

## Tooling

Tooling (Rust, `protoc`, `cargo-shear`, `actionlint`, `shellcheck`) is managed by [mise](https://mise.jdx.dev).

```sh
mise install          # install tools
mise run build        # debug build
mise run test         # run tests
mise run fmt          # format with nightly rustfmt
mise run lint:deps    # check for unused dependencies
mise run lint:actions # lint GitHub Actions workflows
mise run test:ci      # fmt check, clippy, unused deps, workflow lint, tests, and build
```

Run `mise run test:ci` before committing.

## Running locally

Start the server in one terminal and the client in another:

```sh
export WATCHIT_JACKETT_API_KEY=...   # from the Jackett dashboard
export WATCHIT_DELUGE_PASSWORD=...   # Deluge Web UI password
mise run server                      # listens on 127.0.0.1:50051
mise run client ubuntu               # opens the TUI and searches for "ubuntu"
```

The server expects Jackett at `http://localhost:9117` and the Deluge Web UI at `http://localhost:8112`. Set
`JACKETT_URL` or `DELUGE_URL` to point it somewhere else.

Both tasks first run `mise run certs:dev`, which generates dev certs in `hack/certs` (gitignored) for `localhost` and
`127.0.0.1`. Existing certs are left alone. To regenerate them, for example after they expire, delete the directory.
