# Watchit

Search for torrents and manage downloads from a terminal on your laptop, while [Jackett](https://github.com/Jackett/Jackett)
and [Deluge](https://deluge-torrent.org) keep running on a remote machine. A small server sits next to them, and the
client talks to it over mutual TLS (mTLS).

## Features

- Search your Jackett indexers from a terminal UI, with seeders, leechers, size, and date for each result.
- Send a result to Deluge with one key, after a confirmation prompt.
- Watch every torrent in Deluge on a Downloads tab, in-progress ones first, refreshed every 2 seconds.
- Keep traffic private with mTLS and a CA you own. The client's `certs` command creates everything you need.

## Installation

You need Rust 1.99 or newer and [`protoc`](https://protobuf.dev/installation/). From a checkout of this repo:

```sh
cargo install --locked --path crates/server   # installs `server`
cargo install --locked --path crates/client   # installs `client`
```

Install the server on the machine that runs Jackett and Deluge, and the client on your laptop.

## Getting started

### 1. Create certs

Run these on your laptop. Everything is written to `./certs` (change it with `--certs-dir`). Existing files are never
overwritten, and keys are written with `0600` permissions.

```sh
# Create the CA (valid for 10 years)
client certs ca

# Issue the server cert (valid for 1 year). Pass every DNS name and IP you
# will use to reach the server. Add `--san localhost` for local testing.
client certs server --san myhost.example --san 10.0.0.5

# Issue a client cert (valid for 1 year)
client certs client --name laptop
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

### 2. Start the server

The server searches through Jackett's Torznab API and adds torrents through the Deluge Web UI. The Jackett API key is
read from `WATCHIT_JACKETT_API_KEY` and the Deluge password from `WATCHIT_DELUGE_PASSWORD`, to keep them out of shell
history and `ps`.

```sh
export WATCHIT_JACKETT_API_KEY=...   # from the Jackett dashboard
export WATCHIT_DELUGE_PASSWORD=...   # Deluge Web UI password
server --jackett-url http://localhost:9117   # listens on 0.0.0.0:50051, reads certs from ./certs
server --jackett-url http://localhost:9117 --host 10.0.0.5 --port 6000 --certs-dir /etc/watchit
```

### 3. Connect the client

```sh
client --host myhost.example ubuntu          # host must match a --san on the server cert
client --host 10.0.0.5 --port 6000 --certs-dir ~/.config/watchit ubuntu
```

The search term is optional; leave it off to start with an empty search bar.

## Using the client

The client opens a TUI with two tabs. Search has a search bar and a results table. Downloads lists every torrent in
Deluge, in-progress ones first, and refreshes every 2 seconds while it's open.

| Key                 | Search bar      | Results               | Downloads             |
| ------------------- | --------------- | --------------------- | --------------------- |
| `Tab`/`Shift-Tab`   | switch tabs     | switch tabs           | switch tabs           |
| `Enter`             | run the search  | -                     | -                     |
| `/`                 | (types `/`)     | go to search bar      | -                     |
| `j`/`k`, arrows     | -               | move selection        | move selection        |
| `g`/`G`, Home/End   | -               | jump to top/bottom    | jump to top/bottom    |
| `d`                 | (types `d`)     | send to Deluge        | -                     |
| PgUp/PgDn           | -               | move 10 rows          | move 10 rows          |
| `Esc`               | go to results   | quit                  | quit                  |
| `q`                 | (types `q`)     | quit                  | quit                  |
| `Ctrl-C`            | quit            | quit                  | quit                  |

`d` asks before sending. In that prompt, `y`/`Enter` sends the torrent and `n`/`Esc` cancels. Other keys are
ignored.

## Flags

| Flag                | Server default          | Client default |
| ------------------- | ----------------------- | -------------- |
| `--host`            | `0.0.0.0`               | `localhost`    |
| `--port`            | `50051`                 | `50051`        |
| `--certs-dir`       | `certs`                 | `certs`        |
| `--jackett-url`     | required                | -              |
| `--jackett-api-key` | required                | -              |
| `--jackett-indexer` | `all`                   | -              |
| `--deluge-url`      | `http://localhost:8112` | -              |
| `--deluge-password` | required                | -              |

## Security model

Every connection uses mTLS with a private CA that you create:

- The server only accepts clients presenting a cert signed by your CA.
- The client only trusts a server presenting a cert signed by your CA, and checks that the cert is valid for the host
  it dialed.
- Any cert signed by your CA is accepted. There is no revocation, so if a client key is lost or leaked, create a new CA
  and reissue everything.

Keep `ca.key` on your laptop (or somewhere offline). It never needs to be on the server.

## Rotating certs

Leaf certs expire after a year. To renew, delete the old pair and issue a new one with the same CA:

```sh
rm certs/server.pem certs/server.key
client certs server --san myhost.example
```

If you get `certificate not valid for name`, the client's `--host` isn't one of the server cert's SANs. Reissue the
server cert with the right `--san` values.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md).

## License

MIT. See [LICENSE](LICENSE).
