# Contributing to Tayga

Issues and pull requests are welcome. Before a larger change, open an [issue](https://github.com/softberries/tayga/issues) to discuss it, so that nobody spends time on something that will not be merged.

## Contributor License Agreement

Tayga is offered under the AGPLv3 and under a commercial license (see [LICENSING.md](LICENSING.md)). To make that possible, every contributor signs the [Tayga Individual Contributor License Agreement](https://gist.github.com/softberries/3c14d313877887e1afdeaff78af08e09) before a pull request can be merged.

- **How to sign.** When you open your first pull request, CLA assistant comments on it with a link. Sign in with GitHub and accept the agreement; the check on the pull request then passes.
- **Once is enough.** The signature covers your past and future contributions. You are asked again only if the text of the agreement changes.
- **What it says.** You keep the copyright in your work and grant SOFTBERRIES Krzysztof Grajek a license to use and relicense it, including under a commercial license. In return, every contribution that is included stays available under the AGPLv3 or another OSI-approved license. Read the full text before you sign.
- **Work for an employer.** If your employer has rights in what you write, make sure you are allowed to contribute it under the agreement before you sign.

Questions about the agreement: [hello@softberries.dev](mailto:hello@softberries.dev).

## Building and testing

The tools you need and every developer command are in [From source](https://softberries.github.io/tayga/install/from-source/): Rust 1.98 or newer, CMake and a C toolchain for librdkafka, Docker with Compose v2, and Node 24 or newer for the web app.

| What | Command |
|---|---|
| Unit tests | `cargo test --workspace` |
| Integration tests (starts ClickHouse and Redpanda in Docker) | `make it` |
| End-to-end scenarios against the OpenTelemetry demo | `make up`, then `make e2e` |
| Web app lint, typecheck, unit tests | `npm run lint`, `npm run typecheck`, `npm test` in `ui/` |

## Before you open a pull request

CI runs these on every pull request; run them locally first:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

If you changed the web app, also run `npm run lint`, `npm run typecheck` and `npm test` in `ui/`. If you changed the Helm chart, the Compose bundle or the scripts, CI also runs `helm lint`, `docker compose config` and `shellcheck` on them.

- Keep a pull request to one change, and explain what it does and why.
- Add or update tests for behaviour you change.
- Update the documentation in `site/` when you change something users see.
