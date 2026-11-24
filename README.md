![Fennec CLI 1.0.0 — one command for the Fennec suite: a thin, dependency-free Rust launcher that dispatches straight to the native engine binary. Rust edition 2024, entry points fennec · fennec-rs, 81 tests, Windows macOS Linux, MIT license.](docs/hero.png)

---

## At a glance

| Tests | Binaries built | Suite subcommands | `fennec --version` cold start | CI matrix |
|:---:|:---:|:---:|:---:|:---:|
| 81 in 3 files | 2 (`fennec`, `fennec-rs`) | 6 | 10.6 ms median, 2.2 MB binary | 3 OS × cargo test + build + smoke |

One crate, no subfolders: `src/launcher.rs` is the product command, `src/glue.rs` is the
measured Rust form of the suite glue, and `src/bin/` gives them their two names. Counts
come from `cargo test --release` in this checkout and `.github/workflows/cli.yml`; the cold
start is 15 samples of the release binary on this machine (min 9.6 ms, one 113 ms outlier).
No inference code lives here: the engine is [Fennec](https://github.com/Liso7150/fennec),
a single Rust binary. `fennec` is the only command meant for a shell; `fennec-rs` is the
internal name the core bridge spawns.

## Dispatch

```
 fennec <args>
      │
      ├─ suite verb? ──► setup · update · diagnose · preflight · acp · outlets
      │                  (preflight is answered in Rust; setup / acp / outlets
      │                   hand the process to fennec-core)
      │
      └─ otherwise ──► probe the configured engine root, in order:
                         1. target/release/fennec(.exe)  ─► run the binary directly
                         2. target/debug/fennec(.exe)    ─► run the binary directly
                         3. ./fennec(.exe)               ─► run the binary directly
                       argv unchanged · env seeded · exit code preserved
```

Nothing found on PATH is redispatched - only the configured engine root is probed - and
relative arguments stay relative to your shell. `--help` and `--version` work without an engine checkout.

## Install

The crate path-depends on the `fennec-core` crates, so check the two repos out side by side.
Requires Rust (cargo on PATH):

```bash
git clone https://github.com/Liso7150/fennec-core
git clone https://github.com/Liso7150/fennec-cli
cd fennec-cli && cargo install --path . --locked   # fennec and fennec-rs onto PATH
```

Or just `cargo build --release` and use `target/release/`.

> [!NOTE]
> This is a source install, not a claim that the crate is published on crates.io. Use
> [fennec-installer](https://github.com/Liso7150/fennec-installer) to obtain and configure
> the engine itself. Models, inference binaries and GPU drivers are not bundled here.

## Suite commands

| Command | What it does |
|---|---|
| `fennec setup [--yes]` | detect Python/engine/models, write `~/.fennec/config.json` |
| `fennec update [--binary] [--experimental]` | install a verified launcher release when available, then fast-forward the engine and installed suite checkouts unless `--binary` is set |
| `fennec diagnose` | read-only: config, interpreter, engine, model, suite, TokenEconomizer |
| `fennec preflight --model <model-dir>` | weights + peak KV cache against this machine's memory (`--ctx`, `--kv-bytes`, `--json`) |
| `fennec acp` | speak ACP v1 on stdio, for editors |
| `fennec acp --check` | report what the ACP v1 agent advertises |
| `fennec outlets` | status of the optional browser outlets (Fennec's own loop / OpenBot) |
| `fennec --help` / `--version` | launcher help and version, no checkout needed |

`setup`, `acp` and `outlets` hand the process to the separate
[`fennec-core`](https://github.com/Liso7150/fennec-core) service, which must be importable —
either installed or present as a sibling checkout. `preflight` is answered by the Rust
`fennec-preflight` crate in the same binary.

Core serves the canonical supervised PWA/API on `127.0.0.1:8787`. The separate Rust/Axum
`fennec-agent` binds exactly to `127.0.0.1:8788`, unless `FENNEC_AGENT_ADDR` overrides it, and
advertises `chat-only`. It is auxiliary chat and integration infrastructure, never a coding
fallback to core. `diagnose` and `outlets` take no extra arguments.

> [!TIP]
> `fennec diagnose` is this launcher's own read-only check of the *suite*. `fennec doctor`
> is the *engine's* hardware and backend report. They answer different questions.

## Engine dispatch

Every other argument list is passed straight through, unchanged:

```bash
fennec serve --model <model-dir> --auto-tier --threads 8 --host 127.0.0.1 --port 8184
fennec run   --model <model-dir> --prompt "Explain MoE routing"
fennec plan  --model <model-dir>      # show device placement
fennec probe                          # enumerate devices and backends
fennec doctor                         # engine-side hardware/backend health
fennec decode --model <model-dir> --window 40 --tokens 32 --json
fennec bench --model <model-dir>
fennec quant --model <model-dir>
fennec info
```

Run `fennec --help` inside a configured checkout for the engine's full command and flag
list. Exit codes are preserved, so scripts and CI can trust the result.

## Configuration

The launcher reads `FENNEC_CONFIG` or `~/.fennec/config.json`. Without configuration it
uses `~/Fennec` on every platform; the installer's chosen root or `FENNEC_HOME` takes precedence.

| Config key | Environment override |
|---|---|
| `engine_root` | `FENNEC_HOME` |
| `suite_root` | `FENNEC_SUITE` |
| `python` (interpreter for the fennec-core handoff) | `FENNEC_PYTHON` |
| `model` | `FENNEC_MODEL` |
| `cache_root` | `FENNEC_CACHE` |
| `backend` | — |

When dispatching, it also seeds the environment with the `FENNEC_*` knob names the child
reads. Existing values are never overwritten.

| Variable | Set from |
|---|---|
| `FENNEC_MODEL` | `FENNEC_MODEL` or config `model` |
| `FENNEC_VULKAN` | config `backend` (`vulkan` → `1`, `cpu` → `0`) |
| `HF_HOME` | `<cache_root>/huggingface` |
| `PIP_CACHE_DIR` | `<cache_root>/pip` |

## The measured glue: `fennec-rs`

The other binary in this crate is the runnable form of the ported suite glue. It is not a
user command; `rust_bridge.py` spawns `fennec-rs serve` and speaks NDJSON to it,
and `benchmarks/bench.py --impl rust` measures it.

```bash
fennec-rs --help                      # the whole surface
fennec-rs compact <transcript.json>   # bounded context, code points not UTF-16
fennec-rs urlgate <url>...            # allow/block, one line per url
fennec-rs elide <limit>               # stdin, head/tail elided to N characters
fennec-rs redact                      # stdin, Fennec's own redaction
fennec-rs preflight [DIR] --json      # the same budget the launcher's verb answers
fennec-rs serve                       # the NDJSON bridge fennec-core consumes (ping,
                                      # compact, urlgate, elide, pieces, exit)
```

## Update

```bash
fennec update                  # verified launcher release, then engine + suite checkouts
fennec update --binary         # launcher release only
fennec update --experimental   # report trunk, then update checkouts; no commit binary is installed
```

For a newer stable release with a binary matching the current OS and architecture, the updater
requires the release's `SHA256SUMS` manifest. Installation is rejected if the manifest, the
exact asset entry, or its digest is missing, malformed, duplicated, or does not match the
download. No unverified binary is staged or applied. A successful binary replacement requires a
restart.

A source-only release, or one without an asset for this platform, falls through to the checkout
update. `--binary` disables that second step. The update fast-forwards the ten suite checkouts
(`fennec-core`, `fennec-cli`, `fennec-desktop`, `fennec-coding-agent`, `fennec-ui`,
`fennec-economy`, `fennec-jetbrains`, `fennec-installer`, `fennec-finetune`, and this
environment-setup hub) plus the engine checkout, using `git pull --ff-only`.
`fennec-ui` and `fennec-economy` move with their consumers. Engine updates fetch only
`Liso7150/fennec`; the upstream OpenBot outlet is never pulled.

The engine checkout resolves from `FENNEC_HOME`, then `engine_root`, then `~/Fennec`. When its
source moves, the command reports that the built binary is behind and needs
`cargo build --release`. Dirty or diverged work is reported, never reset. Source installs move
the executable only after that rebuild.

| Upstream | How to update it deliberately |
|---|---|
| [JustVugg/colibri](https://github.com/JustVugg/colibri) (credited base; Fennec replaced it and never launches it) | Not updated by Fennec. Maintain a separate legacy checkout only deliberately; the hub's inference update fetches Fennec, not Colibri. |
| [lyogavin/airllm](https://github.com/lyogavin/airllm) (fine-tuning) | `python -m pip install -U airllm` |

## Tests

```bash
cargo test --release          # 81 checks in tests/launcher.rs, tests/cli_surface.rs and tests/updater.rs
```

Everything is hermetic: no network access, no engine checkout required, and the process,
PATH and home-directory seams are injected rather than read from the machine. CI
(`.github/workflows/cli.yml`) runs the suite on Windows, macOS and Ubuntu with `fennec-core`
checked out beside the crate, builds the release binaries, then smoke-tests
`fennec --help`, `fennec --version` and `fennec-rs --help`.

## Credits

Engine: [Liso7150/fennec](https://github.com/Liso7150/fennec) — one Rust engine replacing
fox, Colibri and rabbit. Base idea: [JustVugg/colibri](https://github.com/JustVugg/colibri) —
device-tiered MoE inference. Fine-tuning: [AirLLM](https://github.com/lyogavin/airllm).
Part of the [Fennec suite](https://github.com/Liso7150/fennec-environment-setup).
