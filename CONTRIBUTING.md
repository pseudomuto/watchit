# Contributing

## Layout

| Crate    | Binary           | Purpose                                                                          |
| -------- | ---------------- | -------------------------------------------------------------------------------- |
| `server` | `watchit-server` | gRPC server                                                                      |
| `client` | `watchit`        | TUI for searching and watching downloads, plus `certs` for generating mTLS certs |
| `shared` | -                | Generated protobuf/gRPC code from `proto/`, and mTLS cert generation/loading     |

## Tooling

Tooling (Rust, `protoc`, `cargo-shear`, `actionlint`, `shellcheck`) is managed by [dev](https://github.com/pseudomuto/dev)
from `dev.yaml`.

```sh
dev up           # install tools
dev build        # debug build
dev test         # run tests
dev fmt          # format with nightly rustfmt
dev lint:deps    # check for unused dependencies
dev lint:actions # lint GitHub Actions workflows
dev test:ci      # fmt check, clippy, unused deps, workflow lint, tests, and build
```

Run `dev --help` to list every command.

Run `dev test:ci` before committing.

## Running locally

Start the server in one terminal and the client in another:

```sh
export WATCHIT_JACKETT_API_KEY=...   # from the Jackett dashboard
export WATCHIT_DELUGE_PASSWORD=...   # Deluge Web UI password
dev server                           # listens on 127.0.0.1:50051
dev client ubuntu                    # opens the TUI and searches for "ubuntu"
```

The server expects Jackett at `http://localhost:9117` and the Deluge Web UI at `http://localhost:8112`. Set
`JACKETT_URL` or `DELUGE_URL` to point it somewhere else.

Both commands first run `dev certs:dev`, which generates dev certs in `hack/certs` (gitignored) for `localhost` and
`127.0.0.1`. Existing certs are left alone. To regenerate them, for example after they expire, delete the directory.
