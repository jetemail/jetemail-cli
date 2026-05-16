# Contributing

Thanks for the interest in `jetemail-cli`. The project is small enough that
process is light — open an issue for anything non-trivial first, otherwise PRs
welcome.

## Development

You'll need Rust 1.74+ ([rustup.rs](https://rustup.rs)).

```sh
git clone https://github.com/jetemail/jetemail-cli
cd jetemail-cli
cargo build
cargo test
```

To run the binary without installing it:

```sh
cargo run -- whoami
cargo run -- outbound logs --tail
```

Set `JETEMAIL_API_KEY` in your shell or run `cargo run -- login` first.

## Before you push

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

CI runs the same three checks on every PR.

## Project layout

```
src/
├── main.rs             entry point
├── cli.rs              top-level `clap` parser, dispatch
├── client.rs           HTTP client + auth handling
├── config.rs           ~/.config/jetemail/config.toml loader
├── output.rs           TTY-aware output (tables / JSON / spinners)
├── tui/                ratatui-based live log tail
│   ├── mod.rs
│   └── logs.rs
└── commands/
    ├── auth.rs         login / logout / whoami
    ├── doctor.rs
    ├── completion.rs
    ├── config_cmd.rs
    ├── email.rs
    ├── inbound/        inbound subcommands
    ├── outbound/       outbound subcommands
    ├── webhooks.rs     webhook subscriptions, query, replay
    └── util.rs         body/field helpers shared across commands
```

Every endpoint in [the JetEmail OpenAPI](https://api.jetemail.com/openapi.json)
is mapped to a subcommand. When new endpoints land, the convention is:

- `list` / `get <id>` / `create` / `update <id>` / `delete <id>` for resources
- `--body-json @file.json` and repeated `--field key=value` for advanced bodies
- Hidden `add` alias on any `create` command if it reads more naturally

## Pull request guidelines

- Match the existing style. No new abstractions unless they're used in 2+ places.
- Keep `cargo clippy -- -D warnings` clean.
- If you add a new command, add an example to the README.
- Don't add error handling, retries, or "what if" branches unless there's an
  actual scenario.

## Cutting a release

1. Bump `version` in `Cargo.toml`.
2. `git commit -am "release v0.x.y"` then `git tag v0.x.y`.
3. `git push origin main --tags` — the `release.yml` workflow takes over and
   produces signed archives for every supported platform plus a Homebrew tap
   update.

## Reporting issues

Please include:

- `jetemail --version`
- OS / shell
- The exact command and (redacted) output
- Output of `jetemail doctor` if it's auth-related
