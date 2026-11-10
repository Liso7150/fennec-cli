"""Launch the configured Colibri checkout without redispatching through PATH."""
from __future__ import annotations

import json
import os
import subprocess
import sys
from importlib import metadata
from pathlib import Path

HELP = """\
coli - launcher for the Colibri engine CLI

Usage:
  coli [engine arguments...]     dispatch to the configured checkout's c/coli
  coli --help                    show this help (works without a checkout)
  coli --version                 show the wrapper version

The wrapper finds the engine via COLIBRI_HOME or engine_root in
~/.colibri/config.json (defaults: D:/Colibri on Windows, ~/Colibri elsewhere).
Run `coli --help` inside a checkout for the engine's full command list, or see
README.md. This package ships no inference binaries or model weights.
"""


def version() -> str:
    try:
        return metadata.version("colibri-cli")
    except metadata.PackageNotFoundError:
        return "1.0.0+source"


def load_config() -> dict:
    configured = os.environ.get("COLIBRI_CONFIG")
    path = Path(configured).expanduser() if configured else Path.home() / ".colibri" / "config.json"
    if not path.is_file():
        if os.environ.get("COLIBRI_CONFIG"):
            raise ValueError(f"configured file does not exist: {path}")
        return {}
    config = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(config, dict):
        raise ValueError(f"configuration must be a JSON object: {path}")
    return config


def engine_entry(config: dict) -> tuple[Path, Path]:
    configured = os.environ.get("COLIBRI_HOME") or config.get("engine_root")
    candidates = [Path(configured).expanduser()] if configured else []
    if not configured:
        if os.name == "nt":
            candidates.append(Path("D:/Colibri"))
        candidates.append(Path.home() / "Colibri")
    for home in candidates:
        for relative in ("c/coli", "colibri/cli.py"):
            entry = home / relative
            if entry.is_file():
                return home.resolve(), entry.resolve()
    raise ValueError(
        "Colibri engine checkout not found. Run the Colibri environment installer "
        "or set COLIBRI_HOME to the checkout containing c/coli. "
        "The CLI wrapper does not include inference binaries or model weights."
    )


def launch_command(config: dict, arguments: list[str]) -> list[str]:
    home, entry = engine_entry(config)
    configured_python = os.environ.get("COLIBRI_PYTHON") or config.get("python")
    if configured_python:
        python = Path(configured_python).expanduser()
        if not python.is_file():
            raise ValueError(f"configured Python interpreter does not exist: {python}")
    else:
        python = home / ".venv" / ("Scripts/python.exe" if os.name == "nt" else "bin/python")
        if not python.is_file():
            python = Path(sys.executable)
    return [str(python.resolve()), str(entry), *arguments]


def main():
    arguments = sys.argv[1:]
    if arguments[:1] in (["--version"], ["-V"]):
        print(f"coli {version()}")
        raise SystemExit(0)
    try:
        config = load_config()
        command = launch_command(config, arguments)
        env = dict(os.environ)
        model = os.environ.get("COLIBRI_MODEL") or config.get("model")
        if model:
            env.setdefault("COLI_MODEL", str(model))
        if config.get("backend") in ("vulkan", "cpu"):
            env.setdefault("COLI_VULKAN", "1" if config["backend"] == "vulkan" else "0")
        cache = os.environ.get("COLIBRI_CACHE") or config.get("cache_root")
        if cache:
            env.setdefault("HF_HOME", str(Path(cache) / "huggingface"))
            env.setdefault("PIP_CACHE_DIR", str(Path(cache) / "pip"))
        result = subprocess.run(
            command, env=env, stdin=sys.stdin, stdout=sys.stdout, stderr=sys.stderr,
            creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0,
        )
    except (OSError, ValueError) as exc:
        if arguments[:1] in (["-h"], ["--help"], ["help"]):
            print(HELP)
            raise SystemExit(0) from None
        print(f"coli: {exc}", file=sys.stderr)
        raise SystemExit(2) from None
    raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()
