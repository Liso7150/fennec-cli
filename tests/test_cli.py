import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest import mock
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

    def test_bare_python_command_resolves_from_path(self):
        config = {"engine_root": str(self.home), "python": "python"}
        with patch("fennec_cli.shutil.which", return_value=sys.executable) as which:
            command = fennec_cli.launch_command(config, ["--help"])
        which.assert_any_call("python")
        self.assertEqual(command[0], str(Path(sys.executable).resolve()))

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


class SetupTests(unittest.TestCase):
    def test_help_lists_the_setup_command(self):
        self.assertIn("fennec setup", fennec_cli.HELP)

    def test_setup_dispatches_before_the_strict_config_load(self):
        module = mock.Mock()
        module.main.return_value = 0
        with patch.object(fennec_cli, "load_config", side_effect=AssertionError("too early")), \
             patch.object(fennec_cli, "core_module", return_value=module) as resolver, \
             patch.object(sys, "argv", ["fennec", "setup", "--yes"]):
            with self.assertRaises(SystemExit) as result:
                fennec_cli.main()
        self.assertEqual(result.exception.code, 0)
        resolver.assert_called_once_with("setup_wizard", {})
        module.main.assert_called_once_with(["--yes"], config={})


class DiagnoseTests(unittest.TestCase):
    def test_reports_configured_paths_without_starting_anything(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            home = root / "engine"
            (home / "c").mkdir(parents=True)
            (home / "c" / "coli").write_text("", encoding="utf-8")
            env = {"FENNEC_HOME": str(home), "USERPROFILE": str(root), "HOME": str(root)}
            with patch.dict(os.environ, env, clear=True):
                checks = fennec_cli.doctor({"engine_root": str(home), "python": sys.executable})
        found = {name: (ok, detail) for name, ok, detail in checks}
        self.assertTrue(found["python"][0])
        self.assertTrue(found["engine"][0])
        self.assertFalse(found["suite"][0])
        self.assertIn("tokeneconomizer", found)


class PreflightTests(unittest.TestCase):
    """The CLI's whole job here is plumbing: parse, ask core for a plan, print it.

    It must not import fennec_core to prove that. A standalone CLI checkout - CI,
    or any machine that has not installed core - used to error out of this test
    with "fennec preflight needs fennec-core", because it called the real module.
    """

    def test_reports_json_from_a_model_directory(self):
        asked = []

        def plan(model, ctx, kv_bytes=None):
            asked.append((model, ctx, kv_bytes))
            return {"model": model, "ctx": ctx, "kv_bytes": ctx * (kv_bytes or 0),
                    "weights_fit": True}

        module = SimpleNamespace(plan=plan, human=lambda report: [report["model"]])
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "weights.bin").write_bytes(b"x" * 2048)
            (root / "config.json").write_text(
                json.dumps({"num_hidden_layers": 2, "num_key_value_heads": 2, "head_dim": 4}),
                encoding="utf-8")
            lines = []
            with mock.patch.object(fennec_cli, "core_module", return_value=module):
                code = fennec_cli.preflight_report(
                    ["--model", str(root), "--ctx", "16", "--kv-bytes", "100", "--json"],
                    {}, out=lines.append)
        self.assertEqual(code, 0)
        self.assertEqual(asked, [(str(root), 16, 100)])
        report = json.loads("\n".join(lines))
        self.assertEqual(report["model"], str(root))
        self.assertEqual(report["kv_bytes"], 1600)
        self.assertIn("weights_fit", report)

    def test_the_human_path_prints_what_the_core_rendered(self):
        module = SimpleNamespace(plan=lambda model, ctx, kv_bytes=None: {"model": model},
                                 human=lambda report: ["weights fit", "kv 1.6 GB"])
        with mock.patch.object(fennec_cli, "core_module", return_value=module):
            lines = []
            code = fennec_cli.preflight_report(["--model", "M"], {"ctx": 8192},
                                               out=lines.append)
        self.assertEqual(code, 0)
        self.assertEqual(lines, ["weights fit", "kv 1.6 GB"])


class AcpAndOutletTests(unittest.TestCase):
    """The two suite-facing subcommands must not reach the engine or the network."""

    def test_acp_check_prints_what_the_agent_advertises(self):
        calls = []
        module = SimpleNamespace(
            check=lambda: calls.append("check") or {"protocolVersion": 1,
                                                    "methods": ["initialize", "session/new"]},
            main=lambda argv: (calls.append(tuple(argv)), 7)[1])
        with mock.patch.object(fennec_cli, "core_module", return_value=module), \
             mock.patch.object(sys, "stdout", new_callable=io.StringIO) as out:
            self.assertEqual(fennec_cli.acp_report(["--check"]), 0)
        self.assertEqual(json.loads(out.getvalue())["protocolVersion"], 1)
        self.assertEqual(calls, ["check"])

    def test_plain_acp_hands_the_pipe_to_the_core_agent(self):
        module = SimpleNamespace(main=lambda argv: (self.record.append(argv), 0)[1])
        self.record = []
        with mock.patch.object(fennec_cli, "core_module", return_value=module):
            self.assertEqual(fennec_cli.acp_report([]), 0)
        self.assertEqual(self.record, [[]])

    def test_unknown_acp_argument_is_refused_before_the_pipe_opens(self):
        with mock.patch.object(fennec_cli, "core_module",
                               return_value=SimpleNamespace(main=lambda argv: 0)):
            with self.assertRaises(ValueError) as error:
                fennec_cli.acp_report(["--terminal"])
        self.assertIn("fennec acp argument", str(error.exception))

    def test_missing_core_names_the_package_instead_of_tracebacking(self):
        def refuse(name, config=None):
            raise ValueError(f"fennec {name} needs fennec-core")
        with mock.patch.object(fennec_cli, "core_module", side_effect=refuse):
            with self.assertRaises(ValueError) as error:
                fennec_cli.acp_report(["--check"])
        self.assertIn("fennec-core", str(error.exception))

    def test_outlets_report_prints_both_outlet_statuses(self):
        seen = []
        config_module = SimpleNamespace(load_config=lambda: seen.append("load") or {"jev_root": ""})
        outlets_module = SimpleNamespace(Outlets=lambda cfg: SimpleNamespace(
            status=lambda **kwargs: {"jev": {"present": False},
                                     "openbot": {"present": False, "url": "http://127.0.0.1:3001"}}))

        def fake_core(name, config=None):
            return {"config": config_module, "outlets": outlets_module}[name]

        with mock.patch.object(fennec_cli, "core_module", side_effect=fake_core), \
             mock.patch.object(sys, "stdout", new_callable=io.StringIO) as out:
            self.assertEqual(fennec_cli.outlets_report(), 0)
        self.assertEqual(seen, ["load"])
        self.assertEqual(json.loads(out.getvalue())["openbot"]["url"], "http://127.0.0.1:3001")

    def test_a_report_keeps_non_ascii_when_the_terminal_carries_it(self):
        module = SimpleNamespace(check=lambda: {"model": "D:/套件/qwen36-35b"}, main=lambda argv: 0)
        lines = []
        with mock.patch.object(fennec_cli, "core_module", return_value=module):
            self.assertEqual(fennec_cli.acp_report(["--check"], out=lines.append), 0)
        self.assertIn("套件", lines[0])

    def test_a_console_that_cannot_encode_gets_escaped_json_not_a_crash(self):
        printed = []

        def cp1250_sink(text):
            # what print() does on a Windows console stuck on an ANSI code page
            text.encode("cp1250")
            printed.append(text)

        report = {"model": "D:/套件/qwen36-35b", "note": "pong \U0001F600"}
        module = SimpleNamespace(check=lambda: report, main=lambda argv: 0)
        with mock.patch.object(fennec_cli, "core_module", return_value=module):
            self.assertEqual(fennec_cli.acp_report(["--check"], out=cp1250_sink), 0)
        self.assertEqual(len(printed), 1)
        self.assertTrue(printed[0].isascii(), printed[0])
        self.assertEqual(json.loads(printed[0]), report)


class SubcommandDispatchTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        patcher = mock.patch.dict(os.environ, {"HOME": self.temp.name,
                                               "USERPROFILE": self.temp.name}, clear=True)
        patcher.start()
        self.addCleanup(patcher.stop)

    def dispatch(self, argv, modules):
        stdout = io.StringIO()
        with mock.patch.object(fennec_cli, "load_config", lambda: {}), \
             mock.patch.object(fennec_cli, "core_module",
                               side_effect=lambda name, config=None: modules[name]) as core, \
             mock.patch.object(sys, "argv", ["fennec", *argv]), \
             mock.patch.object(sys, "stdout", stdout), \
             mock.patch.object(subprocess, "run") as run:
            with self.assertRaises(SystemExit) as result:
                fennec_cli.main()
        return result.exception.code, stdout, run, core

    def test_acp_reaches_core_and_never_the_engine(self):
        modules = {"acp": SimpleNamespace(check=lambda: {"protocolVersion": 1},
                                         main=lambda argv: 0)}
        code, stdout, run, core = self.dispatch(["acp", "--check"], modules)
        self.assertEqual(code, 0)
        self.assertEqual(json.loads(stdout.getvalue())["protocolVersion"], 1)
        run.assert_not_called()
        self.assertEqual([call.args[0] for call in core.call_args_list], ["acp"])

    def test_outlets_reaches_core_and_never_the_engine(self):
        modules = {"config": SimpleNamespace(load_config=lambda: {"jev_root": ""}),
                   "outlets": SimpleNamespace(Outlets=lambda cfg: SimpleNamespace(
                       status=lambda **kwargs: {"jev": {"present": False}}))}
        code, stdout, run, core = self.dispatch(["outlets"], modules)
        self.assertEqual(code, 0)
        self.assertEqual(json.loads(stdout.getvalue()), {"jev": {"present": False}})
        run.assert_not_called()
        self.assertEqual([call.args[0] for call in core.call_args_list], ["config", "outlets"])

    def test_outlets_takes_no_arguments(self):
        modules = {"config": SimpleNamespace(load_config=lambda: {}),
                   "outlets": SimpleNamespace(Outlets=lambda cfg: SimpleNamespace(
                       status=lambda **kwargs: {}))}
        code, stdout, run, core = self.dispatch(["outlets", "--json"], modules)
        self.assertEqual(code, 2)
        run.assert_not_called()
        self.assertEqual(core.call_count, 0)


if __name__ == "__main__":
    unittest.main()
