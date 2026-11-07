# Watchit

A gRPC server and CLI client. The server runs on a remote machine and the client connects to it from your laptop over
mutual TLS (mTLS).

## Crates

| Crate    | Binaries          | Purpose                                            |
| -------- | ----------------- | -------------------------------------------------- |
| `server` | `server`, `certs` | gRPC server, plus a tool for generating mTLS certs |
| `client` | `client`          | CLI that talks to the server                       |
| `shared` | -                 | Generated protobuf/gRPC code from `proto/`         |

## Development

Tooling (Rust, `protoc`) is managed by [mise](https://mise.jdx.dev).

```sh
mise install          # install tools
mise run build        # debug build
mise run test         # run tests
mise run test:ci      # fmt check, clippy, tests, and build
```

To run locally, start the server in one terminal and the client in another:

```sh
export WATCHIT_JACKETT_API_KEY=...   # from the Jackett dashboard
mise run server                      # listens on 127.0.0.1:50051
mise run client ubuntu               # searches for "ubuntu"
```

The server expects Jackett at `http://localhost:9117`. Set `JACKETT_URL` to point it somewhere else.

Both tasks first run `mise run certs:dev`, which generates dev certs in `hack/certs` (gitignored) for `localhost` and
`127.0.0.1`. Existing certs are left alone. To regenerate them, for example after they expire, delete the directory.

## Security model

Every connection uses mTLS with a private CA that you create:

- The server only accepts clients presenting a cert signed by your CA.
- The client only trusts a server presenting a cert signed by your CA, and checks that the cert is valid for the host
  it dialed.
- Any cert signed by your CA is accepted. There is no revocation, so if a client key is lost or leaked, create a new CA
  and reissue everything.

Keep `ca.key` on your laptop (or somewhere offline). It never needs to be on the server.

## Setting up certs

Run these on your laptop. Everything is written to `./certs` (change it with `--dir`). Existing files are never
overwritten, and keys are written with `0600` permissions.

```sh
# 1. Create the CA (valid for 10 years)
cargo run --bin certs -- ca

# 2. Issue the server cert (valid for 1 year). Pass every DNS name and IP you
#    will use to reach the server. Add `--san localhost` for local testing.
cargo run --bin certs -- server --san myhost.example --san 10.0.0.5

# 3. Issue a client cert (valid for 1 year)
cargo run --bin certs -- client --name laptop
```

Then copy the server's files over:

```sh
ssh myhost.example mkdir -p certs
scp certs/ca.pem certs/server.pem certs/server.key myhost.example:certs/
```

| File         | Laptop | Server |
| ------------ | ------ | ------ |
| `ca.pem`     | yes    | yes    |
| `ca.key`     | yes    | no     |
| `server.pem` | -      | yes    |
| `server.key` | -      | yes    |
| `client.pem` | yes    | no     |
| `client.key` | yes    | no     |

## Running

The server searches through [Jackett](https://github.com/Jackett/Jackett)'s Torznab API, so it needs the Jackett URL
and API key. The key is read from `WATCHIT_JACKETT_API_KEY` to keep it out of shell history and `ps`.

On the server:

```sh
export WATCHIT_JACKETT_API_KEY=...
server --jackett-url http://localhost:9117   # listens on 0.0.0.0:50051, reads certs from ./certs
server --jackett-url http://localhost:9117 --host 10.0.0.5 --port 6000 --certs-dir /etc/watchit
```

On your laptop:

```sh
client --host myhost.example ubuntu          # host must match a --san on the server cert
client --host 10.0.0.5 --port 6000 --certs-dir ~/.config/watchit ubuntu
```

| Flag                | Server default | Client default |
| ------------------- | -------------- | -------------- |
| `--host`            | `0.0.0.0`      | `localhost`    |
| `--port`            | `50051`        | `50051`        |
| `--certs-dir`       | `certs`        | `certs`        |
| `--jackett-url`     | required       | -              |
| `--jackett-api-key` | required       | -              |
| `--jackett-indexer` | `all`          | -              |

## Rotating certs

Leaf certs expire after a year. To renew, delete the old pair and issue a new one with the same CA:

```sh
rm certs/server.pem certs/server.key
cargo run --bin certs -- server --san myhost.example
```

If you get `certificate not valid for name`, the client's `--host` isn't one of the server cert's SANs. Reissue the
server cert with the right `--san` values.
