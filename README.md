# Colibri CLI

`coli` — the command-line interface for the [Colibri](https://github.com/JustVugg/colibri)
local MoE inference engine. This package is a thin launcher; the implementation
lives in the engine repo (`colibri.cli` / `c/coli`).

## Install

From a checked-out suite, install into the engine's virtual environment:

```bash
python -m pip install -e ./colibri-cli
```

This is a source-install command, not a claim that this package is published on PyPI.
Use the environment installer to obtain and configure the Colibri engine separately.
Do not install two different packages providing `coli` into the same environment.

The wrapper reads `COLIBRI_CONFIG` or `~/.colibri/config.json`, with `engine_root`,
`python`, `model`, `cache_root`, and `backend` settings. `COLIBRI_HOME`,
`COLIBRI_PYTHON`, `COLIBRI_MODEL`, and `COLIBRI_CACHE` override those values.
Without configuration it checks `D:/Colibri` on Windows, then `~/Colibri`.
It dispatches the checkout's `c/coli` directly, never the `coli` command on PATH;
relative command arguments remain relative to your current directory.
Models, compiled inference binaries, and GPU drivers are not bundled in this wrapper.

## Commands

```bash
coli serve --model D:/colibri-models/qwen36-35b --auto-tier --policy quality --ctx 32768 --port 8184
coli plan   --model D:/colibri-models/qwen36-35b     # show device placement
coli doctor                                          # hardware/backend health
coli convert --model <hf-checkpoint> --out <container> --ebits 4 --gs 64
coli chat   --model D:/colibri-models/qwen36-35b
coli stop   --port 8184
```

Other subcommands: `build`, `info`, `mirror`, `tune`, `run`, `cluster`, `web`,
`ui`, `setup`, `start`, `status`, `logs`, `mcp`, `bench`, `import`.

`coli --help` and `coli --version` work even without a checkout (`--help`
dispatches to the engine's full help when one is configured).

## Credits

Engine: [JustVugg/colibri](https://github.com/JustVugg/colibri). Fine-tuning uses
[AirLLM](https://github.com/lyogavin/airllm). Part of the
[Colibri suite](https://github.com/Liso7150/colibri-environment-setup).