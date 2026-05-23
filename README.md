# jetemail-cli

The official CLI for [JetEmail](https://jetemail.com). Send transactional email,
manage inbound and outbound domains, control SMTP smarthost users, suppression
lists, API keys, webhooks — every endpoint of the JetEmail API, from the terminal
or your CI pipeline.

[![CI](https://github.com/jetemail/jetemail-cli/actions/workflows/ci.yml/badge.svg)](https://github.com/jetemail/jetemail-cli/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/jetemail/jetemail-cli?sort=semver)](https://github.com/jetemail/jetemail-cli/releases)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)

## Install

### Homebrew (macOS / Linux)

```sh
brew install jetemail/cli/jetemail
```

### Shell installer (macOS / Linux)

```sh
curl -fsSL https://github.com/jetemail/jetemail-cli/releases/latest/download/install.sh | sh
```

### Scoop (Windows)

```powershell
scoop bucket add jetemail https://github.com/jetemail/scoop-bucket
scoop install jetemail-cli
```

### PowerShell installer (Windows)

```powershell
irm https://github.com/jetemail/jetemail-cli/releases/latest/download/install.ps1 | iex
```

### Pre-built binaries

Every release publishes both an archive (binary + `README` + `LICENSE-*`)
and a standalone binary you can download directly:

| Archive (with docs)                                  | Standalone binary                            |
|------------------------------------------------------|----------------------------------------------|
| `jetemail-<ver>-x86_64-pc-windows-msvc.zip`          | `jetemail-<ver>-x86_64-pc-windows-msvc.exe`  |
| `jetemail-<ver>-aarch64-apple-darwin.tar.gz`         | `jetemail-<ver>-aarch64-apple-darwin`        |
| `jetemail-<ver>-x86_64-unknown-linux-gnu.tar.gz`     | `jetemail-<ver>-x86_64-unknown-linux-gnu`    |
| `jetemail-<ver>-aarch64-unknown-linux-gnu.tar.gz`    | `jetemail-<ver>-aarch64-unknown-linux-gnu`   |
| `jetemail-<ver>-x86_64-unknown-linux-musl.tar.gz`    | `jetemail-<ver>-x86_64-unknown-linux-musl`   |

Grab whichever you prefer from
[the latest release](https://github.com/jetemail/jetemail-cli/releases/latest),
rename to `jetemail` (or `jetemail.exe`), and put it on your `PATH`.

### From source

Requires Rust 1.74+ ([rustup.rs](https://rustup.rs)):

```sh
cargo install --git https://github.com/jetemail/jetemail-cli
```

## Quickstart

```sh
jetemail login                                # prompts for your api_… key, validates, saves
jetemail send --to you@example.com \          # alias for `jetemail email send`
              --from noreply@yourdomain.com \
              --subject "Hello"               # body prompted, or pass --html / --text
jetemail outbound domains list                # human-readable table
jetemail doctor                               # self-test if anything's off
```

## Authentication

`jetemail` resolves the API key in this order — first match wins:

1. `--api-key api_…` (and `--transactional-key transactional_…`) flag
2. `JETEMAIL_API_KEY` / `JETEMAIL_TRANSACTIONAL_KEY` environment variables
3. Saved config (`jetemail login` writes here)

> **Avoid passing keys as flags on shared machines.** A key on the command line
> is visible to other local users via `ps`/`/proc/<pid>/cmdline` and is written
> to your shell history and CI logs. Prefer `jetemail login` or the environment
> variables, which are not exposed on the process argument list.

```sh
jetemail login              # interactive
jetemail whoami             # show the active key (masked) and re-validate against the API
jetemail logout             # clear saved credentials
```

For non-interactive contexts (CI, scripts), set `JETEMAIL_API_KEY` and skip `login`.

### Config file

```sh
jetemail config path        # prints the location
jetemail config show        # dumps the TOML (secrets masked; --reveal for full)
jetemail config set api_key api_xxxxxxxx
jetemail config get api_key # masked on a TTY; raw when piped, or with --reveal
jetemail config unset api_key
```

Default locations (override with `JETEMAIL_CONFIG=…`):

- macOS: `~/Library/Application Support/com.JetEmail.jetemail/config.toml`
- Linux: `~/.config/jetemail/config.toml`
- Windows: `%APPDATA%\JetEmail\jetemail\config\config.toml`

On Unix the file is created `0600` (owner read/write only) and its directory is
tightened to `0700`. On Windows there is no portable `chmod`; the file inherits
the per-user `%APPDATA%` ACLs, so protection there relies on standard Windows
account isolation — avoid storing keys in config on a shared/roaming-profile
Windows host.

## Output

`jetemail` adapts to its environment:

- In a **terminal**, `list`-style commands render compact tables; `email send`
  prompts for missing fields, shows a spinner, and prints a friendly success
  line.
- In a **pipe or non-TTY** (`jq …`, CI logs, AI agents), every command emits
  pretty JSON to stdout. Errors and spinners go to stderr so stdout stays clean.

| Flag         | Effect                                                  |
|--------------|---------------------------------------------------------|
| `--json`     | Force JSON output even in a terminal                    |
| `--raw`      | Compact (single-line) JSON                              |
| `-q --quiet` | Suppress spinners and status chatter                    |

Suppression export (`/outbound/suppression/export`) writes raw CSV to stdout.

## Commands

Every JetEmail API endpoint is mapped to a subcommand. Run any command with
`--help` to see flags and examples.

```
jetemail
├── login                      interactive auth setup
├── logout                     clear saved credentials
├── whoami                     show + validate current key
├── doctor                     self-test
├── completion <shell>         tab-completion script
├── email
│   ├── send                   POST   /email                  (also: `jetemail send …`)
│   └── batch                  POST   /email-batch
├── inbound
│   ├── account
│   │   ├── allowlist          list | get | create | update | delete
│   │   ├── blocklist          list | get | create | update | delete
│   │   └── logs               GET    /inbound/account/logs   (supports --tail)
│   ├── domains
│   │   ├── list               GET    /inbound/domains
│   │   ├── create             POST   /inbound/domains        (alias: add)
│   │   ├── delete <uuid>      DELETE /inbound/domains
│   │   ├── check <uuid>       POST   /inbound/domains/{uuid}/check
│   │   └── forward-rules      list | add | get | update | delete
│   ├── filters
│   │   ├── allowlist          list | get | create | update | delete
│   │   └── blocklist          list | get | create | update | delete
│   ├── destinations           list | verify <token> | resend <uuid>
│   ├── logs                   GET    /inbound/logs           (supports --tail)
│   ├── quarantine release <uid>
│   └── settings               get <uuid> | update <uuid>
├── outbound
│   ├── domains                list | create | delete | check | settings
│   ├── logs                   GET    /outbound/logs          (supports --tail)
│   ├── logs uid <uid>         GET    /outbound/logs/uid
│   ├── smarthost              list | create | update | delete
│   ├── suppression            list | create | delete | export | import
│   └── keys                   list | create | update | delete | rotate
├── webhooks
│   ├── list | get | create | update | delete
│   ├── query
│   └── replay
└── config                     path | show | get | set | unset
```

## Examples

### Send an email

```sh
jetemail send \
  --from 'noreply@example.com' \
  --to alice@example.org --to bob@example.org \
  --subject 'Welcome' \
  --html @welcome.html \
  --attach ./invoice.pdf \
  --idempotency-key "$(uuidgen)"
```

### Add and verify an inbound domain

```sh
jetemail inbound domains create \
  --domain inbox.example.com \
  --delivery-type forward

jetemail inbound domains check <uuid>
```

### Create a forwarding rule

```sh
jetemail inbound domains forward-rules add <domain-uuid> \
  --localpart sales \
  --destination 'sales-team@yourcompany.com' \
  --active true
```

### Manage the suppression list

```sh
jetemail outbound suppression create --target user@example.com --reason bounce
jetemail outbound suppression export > suppressions.csv
jetemail outbound suppression import @suppressions.csv
jetemail outbound suppression delete 42
```

### Rotate a transactional API key

```sh
jetemail outbound keys list
jetemail outbound keys rotate transactional_old_token
```

## Live log tail

`jetemail outbound logs --tail` (and the same flag on `inbound logs` / `inbound
account logs`) opens an interactive TUI: status updates in place as messages
move through the pipeline. Press **Enter** for a structured detail view, **`r`**
to see the raw event JSON, **Space** to pause, **`/`** to filter visible rows,
**`q`** to quit.

```sh
jetemail outbound logs --tail
jetemail outbound logs --tail --action BOUNCED_HARD --poll-secs 10
jetemail inbound logs --tail --logtype spam
```

Only entries created after launch are shown — pass `--date-from 0` to include
the existing buffer too. Default poll interval is 5 seconds; raise it with
`--poll-secs N` if you're running long sessions.

## Body & field escape hatches

Every command that takes a JSON body also accepts:

- `--body-json <source>` — the full body. `source` can be a literal JSON
  string, `@path/to/file.json`, or `-` for stdin.
- `--field key=value` (repeatable) — set or override individual top-level
  fields. The value is parsed as JSON first (`count=5`, `active=true`,
  `tags='["a","b"]'`), falling back to a plain string.

Typed flags (`--from`, `--subject`, …) win over `--body-json` for the same
field, so you can layer overrides on top of a stored template.

## Shell completions

```sh
# zsh
jetemail completion zsh > ~/.zsh/completions/_jetemail
# then in ~/.zshrc:
#   fpath=(~/.zsh/completions $fpath)
#   autoload -U compinit && compinit

# bash
jetemail completion bash > /usr/local/etc/bash_completion.d/jetemail

# fish
jetemail completion fish > ~/.config/fish/completions/jetemail.fish

# powershell
jetemail completion powershell >> $PROFILE
```

## Exit codes

- `0` — success
- `1` — error (network, HTTP non-2xx, validation, etc.)

Errors print the HTTP status and parsed response body to stderr.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). The project is small enough that
process is light — for non-trivial changes please open an issue first.

## License

Dual-licensed at your option under either of:

- [MIT License](LICENSE-MIT)
- [Apache License, Version 2.0](LICENSE-APACHE)

Contributions are accepted under the same dual license.
