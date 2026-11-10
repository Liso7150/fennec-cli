import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import fennec_cli


# Captured before tests strip the environment; CPython 3.10 on Windows needs
# SYSTEMROOT (and friends) to initialize its RNG in child interpreters.
_ESSENTIAL_ENV = {
    key: value for key, value in os.environ.items()
    if key in ("SYSTEMROOT", "WINDIR", "PATH", "TEMP", "TMP", "COMSPEC", "PATHEXT")
} if os.name == "nt" else {}


class LauncherTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.home = self.root / "engine with spaces"
        (self.home / "c").mkdir(parents=True)
        (self.home / "c" / "coli").write_text("", encoding="utf-8")
        self.env = patch.dict(os.environ, {
            "FENNEC_HOME": str(self.home), "USERPROFILE": str(self.root), "HOME": str(self.root),
        }, clear=True)
        self.env.start()
        self.addCleanup(self.env.stop)

    def test_checkout_is_dispatched_directly(self):
        command = fennec_cli.launch_command({}, ["serve", "--model", "relative/model"])
        self.assertEqual(command[1], str((self.home / "c" / "coli").resolve()))
        self.assertEqual(command[2:], ["serve", "--model", "relative/model"])
        self.assertNotEqual(Path(command[0]).stem, "coli")

    def test_missing_configured_engine_never_searches_path(self):
        with patch.dict(os.environ, {"FENNEC_HOME": str(self.root / "missing")}):
            with patch("subprocess.run") as run:
                with self.assertRaises(ValueError):
                    fennec_cli.launch_command({}, [])
                run.assert_not_called()

    def test_config_root_and_python(self):
        with patch.dict(os.environ, {}, clear=True):
            config = {"engine_root": str(self.home), "python": sys.executable}
            self.assertEqual(fennec_cli.launch_command(config, ["--help"])[0], str(Path(sys.executable).resolve()))

    def test_environment_overrides_config(self):
        command = fennec_cli.launch_command({"engine_root": "missing"}, [])
        self.assertEqual(command[1], str((self.home / "c" / "coli").resolve()))

    def test_real_subprocess_preserves_output_and_arguments(self):
        (self.home / "c" / "coli").write_text(
            "import json, sys\nprint(json.dumps(sys.argv[1:]))\nraise SystemExit(3)\n",
            encoding="utf-8",
        )
        result = subprocess.run(
            [sys.executable, "-c", "import fennec_cli; fennec_cli.main()", "--flag", "value with spaces"],
            capture_output=True, text=True, timeout=20, env={**os.environ, **_ESSENTIAL_ENV},
        )
        self.assertEqual(result.returncode, 3, result.stderr)
        self.assertEqual(json.loads(result.stdout), ["--flag", "value with spaces"])

    def test_explicit_invalid_python_fails(self):
        with self.assertRaises(ValueError):
            fennec_cli.launch_command({"python": str(self.root / "missing-python")}, [])

    def test_invalid_json_config_fails(self):
        config = self.root / "config.json"
        config.write_text("[]", encoding="utf-8")
        with patch.dict(os.environ, {"FENNEC_CONFIG": str(config)}):
            with self.assertRaises(ValueError):
                fennec_cli.load_config()

    def test_explicit_missing_config_fails(self):
        with patch.dict(os.environ, {"FENNEC_CONFIG": str(self.root / "absent.json")}):
            with self.assertRaises(ValueError):
                fennec_cli.load_config()

    def test_help_without_checkout_prints_usage(self):
        with patch.dict(os.environ, {"FENNEC_HOME": str(self.root / "missing")}), \
             patch.object(sys, "argv", ["coli", "--help"]), \
             patch("subprocess.run") as run, \
             patch("sys.stdout", new_callable=io.StringIO) as out:
            with self.assertRaises(SystemExit) as result:
                fennec_cli.main()
        self.assertEqual(result.exception.code, 0)
        self.assertIn("Usage:", out.getvalue())
        run.assert_not_called()

    def test_version_without_checkout(self):
        with patch.dict(os.environ, {"FENNEC_HOME": str(self.root / "missing")}), \
             patch.object(sys, "argv", ["fennec", "--version"]), \
             patch("subprocess.run") as run, \
             patch("sys.stdout", new_callable=io.StringIO) as out:
            with self.assertRaises(SystemExit) as result:
                fennec_cli.main()
        self.assertEqual(result.exception.code, 0)
        self.assertTrue(out.getvalue().startswith("fennec "))
        self.assertRegex(out.getvalue(), r"^fennec \d+\.\d+\.\d+")
        run.assert_not_called()

    def test_update_fast_forwards_each_installed_checkout(self):
        suite = self.root / "Suite"
        for name in ("fennec-desktop", "fennec-cli", "fennec-jetbrains"):
            (suite / name / ".git").mkdir(parents=True)
        calls = []

        def runner(command, **kwargs):
            calls.append(command)
            return subprocess.CompletedProcess(command, 0, stdout="Already up to date.\n", stderr="")

        out = io.StringIO()
        with patch.dict(os.environ, {"FENNEC_SUITE": str(suite)}):
            code = fennec_cli.update_suite({}, runner=runner, out=lambda m: out.write(m + "\n"),
                                            err=lambda m: out.write(m + "\n"))
        self.assertEqual(code, 0)
        self.assertEqual([call[2] for call in calls], [str(suite / name) for name in
                         ("fennec-desktop", "fennec-cli", "fennec-jetbrains")])
        self.assertTrue(all(call[1] == "-C" and call[3:] == ["pull", "--ff-only"] for call in calls))
        self.assertIn("JustVugg/colibri", out.getvalue())
        self.assertIn("lyogavin/airllm", out.getvalue())

    def test_update_reports_failures_and_missing_checkouts(self):
        suite = self.root / "Suite"
        (suite / "fennec-core" / ".git").mkdir(parents=True)

        def failing(command, **kwargs):
            return subprocess.CompletedProcess(command, 1, stdout="", stderr="local changes would be overwritten")

        err = io.StringIO()
        with patch.dict(os.environ, {"FENNEC_SUITE": str(suite)}):
            self.assertEqual(fennec_cli.update_suite({}, runner=failing, out=lambda m: None,
                                                      err=lambda m: err.write(m)), 1)
            self.assertIn("fennec-core", err.getvalue())
        with patch.dict(os.environ, {"FENNEC_SUITE": str(self.root / "empty")}):
            self.assertEqual(fennec_cli.update_suite({}, runner=failing, out=lambda m: None,
                                                      err=lambda m: None), 2)

    def test_update_command_dispatches_without_the_engine_checkout(self):
        with patch.object(fennec_cli, "load_config", return_value={}), \
             patch.object(sys, "argv", ["coli", "update"]), \
             patch.object(fennec_cli, "update_suite", return_value=0) as update:
            with self.assertRaises(SystemExit) as result:
                fennec_cli.main()
        self.assertEqual(result.exception.code, 0)
        update.assert_called_once()

    def test_exit_code_environment_and_callers_directory_preserved(self):
        config = {"model": "model-dir", "cache_root": str(self.root / "cache"), "backend": "vulkan"}
        with patch.object(fennec_cli, "load_config", return_value=config), \
             patch.object(sys, "argv", ["coli", "doctor"]), \
             patch("subprocess.run", return_value=subprocess.CompletedProcess([], 7)) as run:
            with self.assertRaises(SystemExit) as result:
                fennec_cli.main()
        self.assertEqual(result.exception.code, 7)
        self.assertNotIn("cwd", run.call_args.kwargs)
        self.assertEqual(run.call_args.kwargs["env"]["COLI_MODEL"], "model-dir")
        self.assertEqual(run.call_args.kwargs["env"]["COLI_VULKAN"], "1")
        self.assertEqual(run.call_args.kwargs["env"]["HF_HOME"], str(self.root / "cache" / "huggingface"))


if __name__ == "__main__":
    unittest.main()
