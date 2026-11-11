"""Launch the configured Fennec checkout without redispatching through PATH."""
from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from importlib import metadata
from pathlib import Path

HELP = """\
fennec - launcher for the Fennec suite and its local inference engine

Usage:
  fennec [engine arguments...]   dispatch to the configured checkout's c/coli
  fennec update                  fast-forward installed Fennec suite checkouts
  fennec diagnose                read-only checks: interpreter, engine, model, suite, TokenEconomizer
  fennec preflight --model DIR   weights + peak KV cache against this machine's memory
  fennec --help                  show this help (works without a checkout)
  fennec --version               show the wrapper version

`coli` is an alias for the same launcher: it is the engine's own entry-point name.
The wrapper finds the engine via FENNEC_HOME or engine_root in
~/.fennec/config.json (default: ~/Colibri on every platform). Pick any drive or
folder with the installer's --root; nothing assumes a D: drive.
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
        return "1.8.0+source"


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
    candidates = [Path(configured).expanduser()] if configured else [Path.home() / "Colibri"]
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


def preflight_module(config: dict | None = None):
    """The core `preflight` module, from the suite checkout when it is not installed."""
    try:
        from fennec_core import preflight
        return preflight
    except ImportError:
        roots = [suite_root(config or {}),
                 Path(__file__).resolve().parent.parent.parent,
                 Path.cwd()]
        looked = []
        for root in roots:
            candidate = root / "fennec-core"
            looked.append(str(candidate))
            if (candidate / "fennec_core" / "preflight.py").is_file():
                sys.path.insert(0, str(candidate))
                from fennec_core import preflight
                return preflight
        raise ValueError(
            "fennec preflight needs fennec-core; install it or keep fennec-core "
            f"next to this checkout (looked in {', '.join(looked)})")


def preflight_report(arguments: list[str], config: dict | None = None, out=print) -> int:
    import argparse
    parser = argparse.ArgumentParser(prog="fennec preflight",
                                     description="Report weights + peak KV cache against this machine")
    parser.add_argument("--model", required=True, help="model directory to inspect")
    parser.add_argument("--ctx", type=int, default=None, help="context length to budget (default: config ctx or 8192)")
    parser.add_argument("--kv-bytes", type=int, default=None,
                        help="override KV bytes per token when config.json has no geometry")
    parser.add_argument("--json", action="store_true", help="machine-readable output")
    args = parser.parse_args(arguments)
    cfg = config or {}
    ctx = args.ctx or int(cfg.get("ctx") or 0) or 8192
    module = preflight_module(cfg)
    report = module.plan(args.model, ctx, kv_bytes=args.kv_bytes)
    if args.json:
        out(json.dumps(report, indent=2))
    else:
        for line in module.human(report):
            out(line)
    return 0


def doctor(config: dict | None = None, environ: dict | None = None) -> list[tuple[str, bool, str]]:
    """Read-only checks: nothing starts, nothing is written."""
    env = dict(os.environ if environ is None else environ)
    cfg = config or {}
    checks: list[tuple[str, bool, str]] = []
    configured = env.get("FENNEC_CONFIG")
    path = Path(configured).expanduser() if configured else Path.home() / ".fennec" / "config.json"
    checks.append(("config", path.is_file(),
                   str(path) if path.is_file() else f"not found: {path}"))
    try:
        command = launch_command(cfg, [])
        checks.append(("python", True, command[0]))
        checks.append(("engine", True, command[1]))
    except (OSError, ValueError) as exc:
        checks.append(("python", False, str(exc)))
        checks.append(("engine", False, str(exc)))
    model = env.get("FENNEC_MODEL") or cfg.get("model")
    if model:
        model_path = Path(model).expanduser()
        checks.append(("model", model_path.is_dir(), str(model_path)))
    root = suite_root(cfg)
    found = [name for name in SUITE_REPOS if (root / name / ".git").is_dir()]
    checks.append(("suite", bool(found), f"{len(found)} checkout(s) under {root}"))
    pointer = Path.home() / ".fennec" / "tokeneconomizer.path"
    core = env.get("FENNEC_TOKENECONOMIZER") or env.get("TE_CORE_PATH")
    if not core and pointer.is_file():
        core = pointer.read_text(encoding="utf-8").strip()
    ready = bool(core) and (Path(core) / "tokeneconomizer" / "plugin" / "hook.py").is_file()
    checks.append(("tokeneconomizer", ready, core or "not configured"))
    return checks


def print_doctor(checks, out=print) -> int:
    for name, ok, detail in checks:
        out(f"{'ok  ' if ok else 'fail'} {name:16s} {detail}")
    return 0 if all(ok for name, ok, _ in checks if name in ("python", "engine")) else 1


def launch_command(config: dict, arguments: list[str]) -> list[str]:
    home, entry = engine_entry(config)
    configured_python = os.environ.get("FENNEC_PYTHON") or config.get("python")
    if configured_python:
        python = Path(configured_python).expanduser()
        if not python.is_file():
            found = shutil.which(configured_python)
            if not found:
                raise ValueError(f"configured Python interpreter does not exist: {python}")
            python = Path(found)
    else:
        python = Path(shutil.which("python") or shutil.which("python3") or sys.executable)
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
        if arguments[:1] == ["diagnose"]:
            if len(arguments) > 1:
                raise ValueError("fennec diagnose takes no extra arguments")
            raise SystemExit(print_doctor(doctor(config)))
        if arguments[:1] == ["preflight"]:
            raise SystemExit(preflight_report(arguments[1:], config))
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
