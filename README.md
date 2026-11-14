![Fennec CLI 1.0.0 — one command for the Fennec suite: a thin, dependency-free launcher that dispatches straight to the native engine binary. Python 3.10+, zero third-party dependencies, entry points fennec and coli, 46 tests, Windows macOS Linux, MIT license.](docs/hero.png)

---

## At a glance

| Tests | Third-party deps | Entry points | Suite subcommands | CI matrix |
|:---:|:---:|:---:|:---:|:---:|
| 46 in 1 file | 0 | 2 (`fennec`, `coli`) | 6 | 3 OS × 3 Python |

Counts from `tests/test_cli.py`, `pyproject.toml` and `.github/workflows/ci.yml`.
No inference code lives here: the engine is [Fennec](https://github.com/Liso7150/fennec),
a single Rust binary. `coli` is installed as an alias because that was the legacy engine's
own entry-point name.

## Dispatch

```
 fennec <args>   (or: coli <args>)
      │
      ├─ suite verb? ──► setup · update · diagnose · preflight · acp · outlets
      │                  (setup / preflight / acp / outlets delegate to fennec-core)
      │
      └─ otherwise ──► probe the configured engine root, in order:
                         1. target/release/fennec(.exe)  ─► run the binary directly
                         2. target/debug/fennec(.exe)    ─► run the binary directly
                         3. ./fennec(.exe)               ─► run the binary directly
                         4. c/coli · colibri/cli.py      ─► python <launcher>  (legacy)
                       argv unchanged · env seeded · exit code preserved
```

An unrelated `coli` on PATH is never redispatched, and relative arguments stay relative to
your shell. `--help` and `--version` work without an engine checkout.

## Install

Requires Python 3.10+. From a checked-out suite:

```bash
python -m pip install -e ./fennec-cli
```

> [!NOTE]
> This is a source install, not a claim that the package is published on PyPI. Use
> [fennec-installer](https://github.com/Liso7150/fennec-installer) to obtain and configure
> the engine itself. Do not install two different packages providing `coli` into the same
> environment. Models, inference binaries and GPU drivers are not bundled here.

## Suite commands

| Command | What it does |
|---|---|
| `fennec setup [--yes]` | detect Python/engine/models, write `~/.fennec/config.json` |
| `fennec update` | fast-forward installed suite checkouts |
| `fennec diagnose` | read-only: config, interpreter, engine, model, suite, TokenEconomizer |
| `fennec preflight --model <model-dir>` | weights + peak KV cache against this machine's memory (`--ctx`, `--kv-bytes`, `--json`) |
| `fennec acp` | speak ACP v1 on stdio, for editors |
| `fennec acp --check` | report what the ACP v1 agent advertises |
| `fennec outlets` | status of the optional Jev / OpenBot web-agent outlets |
| `fennec --help` / `--version` | launcher help and version, no checkout needed |

`setup`, `preflight`, `acp` and `outlets` delegate to
[`fennec-core`](https://github.com/Liso7150/fennec-core), which must be importable — either
installed, or present as a sibling checkout. `update`, `diagnose` and `outlets` take no
extra arguments.

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
tries `~/Fennec`, then `~/Colibri`, on every platform; the installer's chosen root or
`FENNEC_HOME` wins. No drive letter is assumed.

| Config key | Environment override |
|---|---|
| `engine_root` | `FENNEC_HOME` |
| `suite_root` | `FENNEC_SUITE` |
| `python` (Colibri fallback only) | `FENNEC_PYTHON` |
| `model` | `FENNEC_MODEL` |
| `cache_root` | `FENNEC_CACHE` |
| `backend` | — |

When dispatching, it also seeds the environment so both engines agree. Existing values are
never overwritten.

| Variable | Set from |
|---|---|
| `FENNEC_MODEL`, `COLI_MODEL` | `FENNEC_MODEL` or config `model` |
| `COLI_VULKAN` | config `backend` (`vulkan` → `1`, `cpu` → `0`) |
| `HF_HOME` | `<cache_root>/huggingface` |
| `PIP_CACHE_DIR` | `<cache_root>/pip` |

## Update

```bash
fennec update    # suite checkouts only; prints the upstream credits afterwards
```

Fast-forwards the eight suite repos plus the environment-setup hub checkout with
`git pull --ff-only`. Dirty or diverged work is **reported, never reset**. The engine is
[Liso7150/fennec](https://github.com/Liso7150/fennec); build it with `cargo build --release`
in its checkout. The upstream projects stay credited and are never pulled by this command:

| Upstream | How to update it deliberately |
|---|---|
| [JustVugg/colibri](https://github.com/JustVugg/colibri) (legacy fallback) | `git fetch origin`, rebase, re-apply the [fennec-patch](https://github.com/Liso7150/fennec-patch) commits — or the hub's `setup.py fetch --features inference --update`. Fennec needs no patches. |
| [lyogavin/airllm](https://github.com/lyogavin/airllm) (fine-tuning) | `python -m pip install -U airllm` |

## Tests

```bash
python -m unittest discover -s tests -v     # 46 checks, hermetic
```

`subprocess.run`, `shutil.which` and `Path.home` are mocked; no network access occurs. CI
runs them on Python 3.10, 3.12 and 3.14 across Ubuntu, Windows and macOS, then installs the
package and smoke-tests `fennec --help` and `coli --version`; a separate job builds the
sdist/wheel with `python -m build`.

## Credits

Engine: [Liso7150/fennec](https://github.com/Liso7150/fennec) — one Rust engine replacing
fox, Colibri and rabbit. Base idea: [JustVugg/colibri](https://github.com/JustVugg/colibri) —
device-tiered MoE inference. Fine-tuning: [AirLLM](https://github.com/lyogavin/airllm).
Part of the [Fennec suite](https://github.com/Liso7150/fennec-environment-setup).