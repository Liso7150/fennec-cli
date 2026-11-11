# Fennec CLI

`fennec` — the command-line interface for the Fennec suite and its local MoE
inference engine. The engine's base idea is
[Colibri](https://github.com/JustVugg/colibri); this package is a thin launcher and
the implementation lives in the engine repo (`colibri.cli` / `c/coli`). `coli`
stays available as an alias because that is the engine's own entry-point name.

## Features

- One `fennec` command for `serve`, `build`, `plan`, `doctor`, `bench` and the rest of the engine CLI; `coli` behaves identically.
- `fennec update` fast-forwards every installed suite checkout with `git pull --ff-only`; dirty or diverged checkouts are reported, never reset.
- `fennec setup` is the headless first-run wizard: it detects Python, the engine and models, then writes the same shared config the desktop app uses. `--yes` accepts defaults, `--dry-run` prints without writing.
- Resolves the engine checkout from `~/.fennec/config.json` / `FENNEC_*`, never from an unrelated `coli` on PATH.
- `--help` and `--version` work without an engine checkout; relative arguments stay relative to your shell.
- Preserves the engine's exit codes, so scripts and CI can trust the result.

## Install

From a checked-out suite, install into the engine's virtual environment:

```bash
python -m pip install -e ./fennec-cli
```

This is a source-install command, not a claim that this package is published on PyPI.
Use the environment installer to obtain and configure the Colibri engine separately.
Do not install two different packages providing `coli` into the same environment.

The wrapper reads `FENNEC_CONFIG` or `~/.fennec/config.json`, with `engine_root`,
`python`, `model`, `cache_root`, and `backend` settings. `FENNEC_HOME`,
`FENNEC_PYTHON`, `FENNEC_MODEL`, and `FENNEC_CACHE` override those values.
Without configuration it uses `~/Colibri` on every platform; the installer's chosen root or `FENNEC_HOME` wins.
It dispatches the checkout's `c/coli` directly, never the `coli` command on PATH;
relative command arguments remain relative to your current directory.
Models, compiled inference binaries, and GPU drivers are not bundled in this wrapper.

## Commands

```bash
fennec setup                                           # interactive: detect, choose, write ~/.fennec/config.json
fennec setup --yes                                     # accept detected defaults (servers, CI)
fennec setup --yes --json                              # machine-readable result
fennec update                                         # fast-forward installed suite checkouts
fennec serve --model ~/fennec-models/qwen36-35b --auto-tier --policy quality --ctx 32768 --port 8184
fennec plan   --model ~/fennec-models/qwen36-35b     # show device placement
fennec doctor                                          # hardware/backend health
fennec convert --model <hf-checkpoint> --out <container> --ebits 4 --gs 64
fennec chat   --model ~/fennec-models/qwen36-35b
fennec stop   --port 8184
```

Other subcommands: `build`, `info`, `mirror`, `tune`, `run`, `cluster`, `web`,
`ui`, `setup`, `start`, `status`, `logs`, `mcp`, `bench`, `import`.

`fennec --help` and `fennec --version` work even without a checkout (`--help`
dispatches to the engine's full help when one is configured).

## Update

```bash
fennec update    # suite checkouts only; prints the upstream credits afterwards
```

The two upstream projects stay credited here and are never pulled by this command:

- **[JustVugg/colibri](https://github.com/JustVugg/colibri)** — the base idea of the
  engine. Update the `~/Colibri` checkout deliberately (`git fetch origin`, then
  rebase and re-apply the private [fennec-patch](https://github.com/Liso7150/fennec-patch)
  commit), or use the hub's `setup.py fetch --features inference --update`.
- **[lyogavin/airllm](https://github.com/lyogavin/airllm)** — streamed LoRA
  fine-tuning. Update it with `python -m pip install -U airllm`.

## Credits

Engine base idea: [JustVugg/colibri](https://github.com/JustVugg/colibri) —
device-tiered MoE inference. Fine-tuning:
[AirLLM](https://github.com/lyogavin/airllm). Part of the
[Fennec suite](https://github.com/Liso7150/fennec-environment-setup).