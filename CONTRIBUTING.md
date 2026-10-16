# Contributing to CacheWitness

Thank you for considering a contribution. This guide covers how to report problems, propose changes, and get a pull request merged.

## Ways to contribute

- **Report a bug or a confusing result.** Include the scenario file, the `cawit` and `cawit-hook` versions, the output of the failing runs, and, for `NOT_REACHED`, what `redis-cli monitor | grep cawit:` showed during a run.
- **Share a case study.** Tried a pattern on an application? Open an issue with the application, where the hook went, and the results, whether the race reproduced or not.
- **Write a hook for another language** following the [hook protocol](docs/book/en/src/reference/hook-protocol.md).
- **Propose a pattern.** Describe the race, the hook point, and at least one fix that passes and one plausible fix that fails.
- **Improve the documentation**, in any of the three languages.

For larger changes, open an issue first so we can agree on the approach.

## Development setup

Requirements: Rust 1.85 or later, Node 18 or later, Docker (for the hook tests and the labs).

```bash
cargo build
cargo test                              # unit tests, scenario loading, doctests
cargo fmt --check
cargo clippy --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

# hook tests need a Redis
docker run -d --name cawit-redis -p 6379:6379 redis:7.2
CAWIT_SYNC_URL=redis://127.0.0.1:6379 npm test --prefix hooks/node
```

The documentation is an mdBook per language under `docs/book/<lang>`. With `mdbook` and `mdbook-mermaid` installed:

```bash
mdbook-mermaid install docs/book/en && mdbook serve docs/book/en
```

## Pull requests

- Keep each pull request focused on one change, with tests for behaviour changes. A fix for a bug should come with a test that fails without it.
- Run the checks above before pushing; CI runs the same.
- Commit messages use [Conventional Commits](https://www.conventionalcommits.org/) prefixes (`feat:`, `fix:`, `docs:`, `chore:`, `test:`), in English.
- User-visible changes go in `CHANGELOG.md` under *Unreleased*.
- Changes to the scenario format or the hook protocol need a note in the changelog and an update to the reference pages, in all three languages if you can (or say which ones you could not do).

## Documentation languages

English is the source of truth. Korean (`ko`) and Simplified Chinese (`zh`) follow the same file layout so the language switcher maps pages one to one. If you change an English page and cannot update the translations, say so in the pull request; a follow-up is fine.

## Lab experiments

The `lab/` directory reproduces the case studies against real applications. Experiments must stay local: the lab scripts refuse to run if the lab network has a route to the outside. Lab patches that modify other projects are licensed under those projects' terms; regenerate them with `lab/make-patch.sh` so the license header is kept.

## License

By contributing, you agree that your contributions are licensed under the Apache License 2.0, except contributions to the lab patches, which are licensed under the license of the project they modify (see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)).
