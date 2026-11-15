# Changelog

All notable changes to fennec-cli, the command-line launcher for the Fennec suite.

The sections below carry the published tag each change actually shipped in
(`Liso7150/fennec-cli`, `v0.1.0` … `v0.9.0`). An earlier revision of this file
numbered the same history on a 1.x/2.x ladder that was never published; those
tags still exist locally (`v1.2.0` … `v2.0.0`) and each one points at the same
commit as a published `v0.x` tag. `v0.2.0` was never released here. Dates are
the tag publish dates.

## [Unreleased] — 1.0.0 in preparation

Nothing at 1.0.0 is published. The newest release here is `v0.9.0`
(2026-11-11T19:29Z). Measured on the working tree: 66 tests passed
(`cargo test --release`, 2026-11-15: 50 in `tests/launcher.rs`, 16 in
`tests/cli_surface.rs`), three console binaries (`fennec`, `coli`,
`fennec-rs`), and no Python package left in the repo.

### Added
- The launcher itself is Rust: `fennec` and `coli` are built from
  `src/launcher.rs`, so the command starts in about 10 ms instead of paying for
  an interpreter
- Rust CLI binary (`fennec-rs`), moved here from fennec-core; CI builds it
  against the fennec-core crates
- CI Rust CLI build expanded to macOS and Ubuntu
- README hero published

### Changed
- Read the model from `FENNEC_MODEL` first, and pin the backend under both the
  `FENNEC_*` and `COLI_*` names
- `fennec update` pulls the hub, which is a suite checkout now
- `fennec-rs` points at `fennec-core/src` and ignores the workspace target dir
- The repo root is the crate: the CPython package `fennec_cli/`, its
  `pyproject.toml`, `tests/test_cli.py` and the Python CI lane are deleted, and
  their behaviour is covered by the 66 Rust checks above. Build it with
  `cargo build --release`; nothing here is pip-installed any more.

## [0.9.0] - 2026-11-11

### Changed
- The command now fronts a Rust core

## [0.8.0] - 2026-11-11

### Added
- `fennec acp` — speak ACP v1 from the terminal
- `fennec outlets` — status of the optional browser outlets
- The preflight check runs as a standalone script, the way CI runs it

## [0.7.1] - 2026-11-11

### Added
- `fennec setup` — headless first-run wizard

## [0.7.0] - 2026-11-11

Version bump only — no change shipped between `0.6.0` and `0.7.0`.

## [0.6.0] - 2026-11-11

### Added
- `fennec preflight` — weights and peak KV cache against this machine's memory

## [0.5.0] - 2026-11-11

### Added
- `fennec diagnose`

## [0.4.0] - 2026-11-11

### Changed
- Launch with python from PATH instead of an engine venv
- Fall back to `~/Colibri` when no engine root is configured

## [0.3.0] - 2026-11-10

### Changed
- Rename the launcher to `fennec`, keep `coli` as the engine alias

## [0.1.0] - 2026-11-10

The first published release. It carries the pre-tag history below.

### Added
- `coli update` for fast-forwarding suite checkouts
- Config-driven launcher with tests and a CI matrix (3 OS × 3 Python)
- Features documented in the README

### Fixed
- `--help` and `--version` work without an engine checkout
- Essential Windows env kept for the subprocess CLI test

### Credits
- Initial commit as colibri-cli (Colibri suite; JustVugg/colibri + lyogavin/airllm)
