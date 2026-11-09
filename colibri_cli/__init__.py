"""Colibri CLI — thin launcher for the engine's `coli` command.

The real CLI lives in the engine repo (colibri.cli / c/coli). This wrapper locates
the engine (COLIBRI_HOME env, default D:/Colibri, or an installed colibri package)
and delegates, so `coli` works from a standalone install.
"""
from __future__ import annotations

import os
import runpy
import shutil
import subprocess
import sys
from pathlib import Path


def _engine_home() -> Path | None:
    env = os.environ.get("COLIBRI_HOME")
    if env and Path(env).exists():
        return Path(env)
    for cand in (Path("D:/Colibri"), Path.home() / "Colibri"):
        if (cand / "c" / "coli").exists() or (cand / "colibri" / "cli.py").exists():
            return cand
    return None


def main():
    home = _engine_home()
    if home is not None:
        cli_py = home / "colibri" / "cli.py"
        if cli_py.exists():
            sys.argv[0] = "coli"
            sys.path.insert(0, str(home))
            runpy.run_path(str(cli_py), run_name="__main__")
            return
    coli = shutil.which("coli")
    if coli:
        raise SystemExit(subprocess.call([coli, *sys.argv[1:]]))
    sys.exit(
        "coli engine not found. Set COLIBRI_HOME to the engine repo, or install it:\n"
        "  python -m pip install git+https://github.com/Liso7150/colibri.git\n"
        "See https://github.com/Liso7150/colibri-environment-setup"
    )


if __name__ == "__main__":
    main()