"""Launch the configured Fennec checkout without redispatching through PATH."""
from __future__ import annotations

import json
import os
import subprocess
import sys
from importlib import metadata
from pathlib import Path

HELP = """\
fennec - launcher for the Fennec suite and its local inference engine

Usage:
  fennec [engine arguments...]   dispatch to the configured checkout's c/coli
  fennec update                  fast-forward installed Fennec suite checkouts
  fennec --help                  show this help (works without a checkout)
  fennec --version               show the wrapper version

`coli` is an alias for the same launcher: it is the engine's own entry-point name.
The wrapper finds the engine via FENNEC_HOME or engine_root in
~/.fennec/config.json (defaults: D:/Colibri on Windows, ~/Colibri elsewhere).
Run `fennec --help` inside a checkout for the engine's full command list, or see
README.md. This package ships no inference binaries or model weights.

Engine base idea: JustVugg/colibri - https://github.com/JustVugg/colibri
Fine-tuning: lyogavin/airllm - https://github.com/lyogavin/airllm
"""

SUITE_REPOS = ("fennec-desktop", "fennec-core", "fennec-cli", "fennec-installer",
               "fennec-finetune", "fennec-jetbrains", "fennec-coding-agent")

UPSTREAM = ("engine base idea JustVugg/colibri (https://github.com/JustVugg/colibri), "
            "fine-tuning lyogavin/airllm (https://github.com/lyogavin/airllm)")


def version() -> str:
    try:
        return metadata.version("fennec-cli")
    except metadata.PackageNotFoundError:
        return "1.4.0+source"


def load_config() -> dict:
    configured = os.environ.get("FENNEC_CONFIG")
    path = Path(configured).expanduser() if configured else Path.home() / ".fennec" / "config.json"
    if not path.is_file():
        if os.environ.get("FENNEC_CONFIG"):
            raise ValueError(f"configured file does not exist: {path}")
        return {}
    config = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(config, dict):
        raise ValueError(f"configuration must be a JSON object: {path}")
    return config


def engine_entry(config: dict) -> tuple[Path, Path]:
    configured = os.environ.get("FENNEC_HOME") or config.get("engine_root")
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
        "Colibri engine checkout not found. Run the Fennec environment installer "
        "or set FENNEC_HOME to the checkout containing c/coli. "
        "The CLI wrapper does not include inference binaries or model weights."
    )


def suite_root(config: dict) -> Path:
    configured = os.environ.get("FENNEC_SUITE") or config.get("suite_root")
    if configured:
        return Path(configured).expanduser()
    if os.name == "nt":
        windows = Path("D:/Fennec-Suite")
        if windows.is_dir():
            return windows
    return Path.home() / "Fennec-Suite"


def update_suite(config: dict, runner=subprocess.run, out=print, err=print) -> int:
    root = suite_root(config)
    directories = [root / name for name in SUITE_REPOS] + [root.parent / "Fennec-Environment-Setup"]
    updated, failed = 0, []
    for directory in directories:
        if not (directory / ".git").is_dir():
            continue
        result = runner(["git", "-C", str(directory), "pull", "--ff-only"],
                        capture_output=True, text=True, encoding="utf-8", errors="replace")
        detail = (result.stdout or "").strip() or (result.stderr or "").strip()
        if result.returncode == 0:
            out(f"ok   {directory.name}: {detail.splitlines()[-1] if detail else 'up to date'}")
            updated += 1
        else:
            failed.append(directory.name)
            out(f"fail {directory.name}: {detail or f'exit {result.returncode}'}")
    if failed:
        err(f"fennec: could not update {', '.join(failed)}; commit or stash local changes first")
        return 1
    if not updated:
        err(f"fennec: no Fennec suite checkouts found under {root}")
        return 2
    out(f"Updated {updated} checkout(s).")
    out(f"Upstream: {UPSTREAM} - neither is touched by this command.")
    return 0


def launch_command(config: dict, arguments: list[str]) -> list[str]:
    home, entry = engine_entry(config)
    configured_python = os.environ.get("FENNEC_PYTHON") or config.get("python")
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
        print(f"fennec {version()}")
        raise SystemExit(0)
    try:
        config = load_config()
        if arguments[:1] == ["update"]:
            if len(arguments) > 1:
                raise ValueError("fennec update takes no extra arguments")
            raise SystemExit(update_suite(config))
        command = launch_command(config, arguments)
        env = dict(os.environ)
        model = os.environ.get("FENNEC_MODEL") or config.get("model")
        if model:
            env.setdefault("COLI_MODEL", str(model))
        if config.get("backend") in ("vulkan", "cpu"):
            env.setdefault("COLI_VULKAN", "1" if config["backend"] == "vulkan" else "0")
        cache = os.environ.get("FENNEC_CACHE") or config.get("cache_root")
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
        print(f"fennec: {exc}", file=sys.stderr)
        raise SystemExit(2) from None
    raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()
