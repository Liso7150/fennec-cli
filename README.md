# Colibri CLI

`coli` — the command-line interface for the [Colibri](https://github.com/JustVugg/colibri)
local MoE inference engine. This package is a thin launcher; the implementation
lives in the engine repo (`colibri.cli` / `c/coli`).

## Install

```bash
python -m pip install colibri-cli
# the engine itself:
python -m pip install git+https://github.com/Liso7150/colibri.git
```

Set `COLIBRI_HOME` to your engine checkout if it is not at `D:/Colibri`.

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

## Credits

Engine: [JustVugg/colibri](https://github.com/JustVugg/colibri). Fine-tuning uses
[AirLLM](https://github.com/lyogavin/airllm). Part of the
[Colibri suite](https://github.com/Liso7150/colibri-environment-setup).