//! The `fennec` launcher: the whole command surface, in the same binary as the
//! ported glue, so the suite has one CLI implementation. It took over from
//! `fennec_cli/__init__.py`, the CPython module that was deleted once
//! `tests/launcher.rs` and `tests/cli_surface.rs` covered its behaviour.
//!
//! Two kinds of work live here. The launcher resolves the engine from the
//! shared config and execs it - native Fennec first, the legacy Colibri
//! `c/coli` launcher under a Python interpreter as the documented fallback -
//! with the configured model, backend and cache carried to the child under both
//! name spellings. The named verbs are the suite side: `update`, `diagnose` and
//! `preflight` are decided here, while `setup`, `acp` and `outlets` reach
//! `fennec_core`, which owns the first-run wizard, ACP v1 and the outlet probes.
//! Those three locate the checkout and hand the process over to the module; they
//! do not reimplement it, which is the same contract the JetBrains sidecar uses
//! for the agent.
use fennec_client::git;
use fennec_client::suite::{self, ConfigRead, Environ, Relative};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use std::process::Command;
#[cfg(windows)]
use std::os::windows::process::CommandExt;

pub const SUITE_REPOS: [&str; 8] = [
    "fennec-desktop",
    "fennec-core",
    "fennec-cli",
    "fennec-installer",
    "fennec-finetune",
    "fennec-jetbrains",
    "fennec-coding-agent",
    "fennec-environment-setup",
];

pub const UPSTREAM: &str = "legacy engine base idea JustVugg/colibri (https://github.com/JustVugg/colibri), \
    fine-tuning lyogavin/airllm (https://github.com/lyogavin/airllm)";

pub const HELP: &str = "\
fennec - launcher for the Fennec suite and the Fennec Rust inference engine

Usage:
  fennec [engine arguments...]   dispatch to the Fennec binary, or legacy c/coli
  fennec update                  fast-forward installed Fennec suite checkouts
  fennec diagnose                read-only checks: interpreter, engine, model, suite, TokenEconomizer
  fennec preflight --model DIR   weights + peak KV cache against this machine's memory
  fennec acp [--check]           speak ACP v1 on stdio (editors), or show what is advertised
  fennec outlets                 show the optional Jev / OpenBot web-agent outlets on this machine
  fennec setup [--yes]           first-run config: detect Python/engine/models, write ~/.fennec/config.json
  fennec --help                  show this help (works without a checkout)
  fennec --version               show the launcher version

`coli` is an alias for the same launcher: it is the legacy engine's entry-point name.
The launcher finds the engine via FENNEC_HOME or engine_root in
~/.fennec/config.json (default: ~/Fennec, then ~/Colibri). Inside that checkout it
runs the native Fennec binary (target/release, target/debug, then the root) with no
Python in front of it, and falls back to Colibri's Python launcher c/coli when
Fennec has not been built (`cargo build --release`). Pick any drive or folder with
the installer's --root; nothing assumes a D: drive.
Run `fennec --help` inside a checkout for the engine's full command list, or see
README.md. This launcher ships no inference binaries or model weights.

Engine base idea: JustVugg/colibri - https://github.com/JustVugg/colibri
Fine-tuning: lyogavin/airllm - https://github.com/lyogavin/airllm
";

pub const NO_ENGINE: &str = "Fennec engine not found. Build it in the engine checkout with \
    `cargo build --release` (target/release/fennec) or run the Fennec environment installer. A \
    legacy Colibri checkout still works as a fallback: set FENNEC_HOME to the checkout containing \
    c/coli. The CLI launcher does not include inference binaries or model weights.";

pub const NEEDS_CORE: &str = "needs fennec-core; install it or keep fennec-core next to this checkout";

/// Hand the process to one `fennec_core` module and propagate its exit code.
/// The checkout, when there is one, travels as `PYTHONPATH`, so the module
/// itself stays a plain argv element.
pub const WIZARD: &str = "import importlib, sys; module = importlib.import_module(sys.argv[1]); \
    raise SystemExit(module.main(sys.argv[2:], config={}))";
pub const ACP_PIPE: &str = "import importlib, sys; module = importlib.import_module(sys.argv[1]); \
    raise SystemExit(module.main([]))";
/// `fennec_core.acp.check()` as indented JSON. Paths and outlet notes are user
/// data, so they stay literal; a console stuck on an ANSI code page that cannot
/// encode them gets escaped JSON instead of a crash in the middle of a report.
/// Each `try` shares its line with its body, because Rust's line continuations
/// drop the leading whitespace that Python's indentation needs.
pub const ACP_CHECK: &str = "import importlib, json, sys\n\
value = importlib.import_module(sys.argv[1]).check()\n\
text = json.dumps(value, indent=2, ensure_ascii=False)\n\
try: print(text)\n\
except UnicodeEncodeError: print(json.dumps(value, indent=2))\n";
pub const OUTLETS_CHECK: &str = "import importlib, json\n\
loaded = importlib.import_module('fennec_core.config').load_config()\n\
value = importlib.import_module('fennec_core.outlets').Outlets(loaded).status()\n\
text = json.dumps(value, indent=2, ensure_ascii=False)\n\
try: print(text)\n\
except UnicodeEncodeError: print(json.dumps(value, indent=2))\n";
/// 0 when the module is importable on this interpreter, 1 when it is not.
pub const PROBE: &str = "import importlib.util as util, sys; \
    raise SystemExit(0 if util.find_spec(sys.argv[1]) else 1)";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Native,
    Colibri,
}

#[derive(Debug)]
pub struct Engine {
    pub home: PathBuf,
    pub entry: PathBuf,
    pub kind: Kind,
}

/// `Path.resolve()` for a path that may not exist yet, with the Windows `\\?\`
/// prefix dropped the way `str(Path(...))` never shows it.
pub fn canonical(path: &Path) -> PathBuf {
    let resolved = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    dunce::simplified(&resolved).to_path_buf()
}

fn field(config: &Map<String, Value>, key: &str) -> Option<String> {
    config.get(key).and_then(Value::as_str).map(str::to_string)
}

/// The shared config, with the launcher's own contract: an explicit
/// `FENNEC_CONFIG` that does not exist is an error, an absent default is empty.
pub fn load_config(environ: &Environ, home: &Path) -> Result<Map<String, Value>, String> {
    let explicit = suite::env_value(environ, "FENNEC_CONFIG").is_some();
    let location = suite::config_file_in(environ, home, Relative::Keep);
    match suite::read_config(&location) {
        ConfigRead::Object(map) => Ok(map),
        ConfigRead::Absent if explicit => {
            Err(format!("configured file does not exist: {}", location.display()))
        }
        ConfigRead::Absent => Ok(Map::new()),
        ConfigRead::NotAnObject => {
            Err(format!("configuration must be a JSON object: {}", location.display()))
        }
        ConfigRead::Unreadable(error) => Err(format!(
            "configuration is not valid JSON: {}: {error}",
            location.display()
        )),
    }
}

/// The engine inside one checkout, and which engine it is. Fennec wins when
/// both are installed; the same relative dirs and names are probed as
/// `fennec_core.engine`.
fn probe_engine(home: &Path) -> Option<(PathBuf, Kind)> {
    let names: &[&str] = if cfg!(windows) {
        &["fennec.exe", "fennec"]
    } else {
        &["fennec"]
    };
    for relative in ["target/release", "target/debug", "."] {
        for name in names {
            let candidate = home.join(relative).join(name);
            if candidate.is_file() {
                return Some((canonical(&candidate), Kind::Native));
            }
        }
    }
    for relative in ["c/coli", "colibri/cli.py"] {
        let candidate = home.join(relative);
        if candidate.is_file() {
            return Some((canonical(&candidate), Kind::Colibri));
        }
    }
    None
}

pub fn engine_target(
    config: &Map<String, Value>,
    environ: &Environ,
    home: &Path,
) -> Result<Engine, String> {
    let configured = suite::env_value(environ, "FENNEC_HOME")
        .map(str::to_string)
        .or_else(|| field(config, "engine_root"));
    let homes: Vec<PathBuf> = match configured {
        Some(value) => vec![suite::expand_in(&value, home, Relative::Keep)],
        None => vec![home.join("Fennec"), home.join("Colibri")],
    };
    for candidate in homes {
        if let Some((entry, kind)) = probe_engine(&candidate) {
            return Ok(Engine { home: canonical(&candidate), entry, kind });
        }
    }
    Err(NO_ENGINE.to_string())
}

/// The interpreter a child runs under, in the launcher's order:
/// `FENNEC_PYTHON`, then the config's `python`, then `python` or `python3` on
/// PATH. A configured value that is neither a file nor a PATH command is
/// reported by [`python_missing`]; with nothing configured and none on PATH the
/// bare name is handed to the spawn, which reports it there.
pub fn python_for(config: &Map<String, Value>, environ: &Environ, home: &Path) -> PathBuf {
    let configured = suite::env_value(environ, "FENNEC_PYTHON")
        .map(str::to_string)
        .or_else(|| field(config, "python"));
    if let Some(value) = configured {
        let path = suite::expand_in(&value, home, Relative::Keep);
        if path.is_file() {
            return canonical(&path);
        }
        if let Some(found) = git::which(&value) {
            return found;
        }
        return path;
    }
    for name in ["python", "python3"] {
        if let Some(found) = git::which(name) {
            return found;
        }
    }
    PathBuf::from("python")
}

pub fn python_is_usable(path: &Path) -> bool {
    path.is_file() || git::which(&path.to_string_lossy()).is_some()
}

#[derive(Debug)]
pub struct Launch {
    pub argv: Vec<String>,
    pub kind: Kind,
}

/// argv for the engine, plus which engine was found. Only the legacy Colibri
/// launcher needs an interpreter in front of it, so only that branch resolves
/// (and validates) one.
pub fn launch_command(
    config: &Map<String, Value>,
    environ: &Environ,
    home: &Path,
    arguments: &[String],
) -> Result<Launch, String> {
    let engine = engine_target(config, environ, home)?;
    let mut argv = Vec::with_capacity(arguments.len() + 2);
    let kind = engine.kind;
    match kind {
        Kind::Native => argv.push(engine.entry.display().to_string()),
        Kind::Colibri => {
            let python = python_for(config, environ, home);
            if !python_is_usable(&python) {
                return Err(python_missing(&python));
            }
            argv.push(canonical(&python).display().to_string());
            argv.push(engine.entry.display().to_string());
        }
    }
    argv.extend(arguments.iter().cloned());
    Ok(Launch { argv, kind })
}

fn python_missing(path: &Path) -> String {
    format!("configured Python interpreter does not exist: {}", path.display())
}

/// The environment pins the child gets, one pair per `env.setdefault`: an
/// exported value is never overwritten, and the engine reads each of these
/// under both spellings with `FENNEC_*` first.
pub fn engine_pins(
    config: &Map<String, Value>,
    environ: &Environ,
) -> Vec<(String, String)> {
    let mut pins = Vec::new();
    let model = suite::env_value(environ, "FENNEC_MODEL")
        .or_else(|| suite::env_value(environ, "COLI_MODEL"))
        .map(str::to_string)
        .or_else(|| field(config, "model"));
    if let Some(model) = model {
        for key in ["FENNEC_MODEL", "COLI_MODEL"] {
            if !environ.contains_key(key) {
                pins.push((key.to_string(), model.clone()));
            }
        }
    }
    if let Some(backend) = field(config, "backend") {
        if matches!(backend.as_str(), "vulkan" | "cpu")
            && !environ.contains_key("FENNEC_VULKAN")
            && !environ.contains_key("COLI_VULKAN")
        {
            let value = if backend == "vulkan" { "1" } else { "0" }.to_string();
            pins.push(("FENNEC_VULKAN".to_string(), value.clone()));
            pins.push(("COLI_VULKAN".to_string(), value));
        }
    }
    let cache = suite::env_value(environ, "FENNEC_CACHE")
        .map(str::to_string)
        .or_else(|| field(config, "cache_root"));
    if let Some(cache) = cache {
        for (key, leaf) in [("HF_HOME", "huggingface"), ("PIP_CACHE_DIR", "pip")] {
            if !environ.contains_key(key) {
                pins.push((
                    key.to_string(),
                    PathBuf::from(cache.clone()).join(leaf).display().to_string(),
                ));
            }
        }
    }
    pins
}

pub fn suite_root(config: &Map<String, Value>, environ: &Environ, home: &Path) -> PathBuf {
    suite::suite_root_in(environ, config, home, Relative::Keep)
}

#[derive(Debug)]
pub struct Update {
    pub code: i32,
    /// `(to_stderr, line)` in the order they were produced, so the report reads
    /// as one stream instead of stdout-then-stderr.
    pub lines: Vec<(bool, String)>,
}

/// Fast-forward every installed suite checkout with `git pull --ff-only`, and
/// credit both upstreams. Nothing is committed, stashed or reset: a dirty
/// checkout is reported, not fixed. Exit 1 means at least one pull failed, 2
/// means no suite checkout was found at all.
pub fn update_suite(
    config: &Map<String, Value>,
    environ: &Environ,
    home: &Path,
    runner: &dyn Fn(&[String]) -> (i32, String, String),
) -> Update {
    let root = suite_root(config, environ, home);
    let parent = root.parent().map(Path::to_path_buf).unwrap_or_else(|| root.clone());
    let directories: Vec<PathBuf> = SUITE_REPOS
        .iter()
        .map(|name| root.join(name))
        .chain(std::iter::once(parent.join("Fennec-Environment-Setup")))
        .collect();
    let mut updated = 0usize;
    let mut failed: Vec<String> = Vec::new();
    let mut lines: Vec<(bool, String)> = Vec::new();
    for directory in directories {
        if !directory.join(".git").is_dir() {
            continue;
        }
        let name = directory.file_name().map(std::ffi::OsStr::to_string_lossy)
            .map(String::from)
            .unwrap_or_else(|| directory.display().to_string());
        let argv = vec![
            "git".to_string(),
            "-C".to_string(),
            directory.display().to_string(),
            "pull".to_string(),
            "--ff-only".to_string(),
        ];
        let (code, stdout, stderr) = runner(&argv);
        let mut detail = stdout.trim();
        if detail.is_empty() {
            detail = stderr.trim();
        }
        if code == 0 {
            let last = detail.lines().next_back().unwrap_or("up to date");
            lines.push((false, format!("ok   {name}: {last}")));
            updated += 1;
        } else {
            lines.push((false, format!(
                "fail {name}: {}",
                if detail.is_empty() { format!("exit {code}") } else { detail.to_string() }
            )));
            failed.push(name);
        }
    }
    let code = if !failed.is_empty() {
        lines.push((true, format!(
            "fennec: could not update {}; commit or stash local changes first",
            failed.join(", ")
        )));
        1
    } else if updated == 0 {
        lines.push((true, format!(
            "fennec: no Fennec suite checkouts found under {}", root.display())));
        2
    } else {
        lines.push((false, format!("Updated {updated} checkout(s).")));
        lines.push((false, format!("Upstream: {UPSTREAM} - neither is touched by this command.")));
        0
    };
    Update { code, lines }
}

#[derive(Debug)]
pub struct Check {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
}

/// Read-only checks: nothing starts, nothing is written.
pub fn doctor(
    config: &Map<String, Value>,
    environ: &Environ,
    home: &Path,
) -> Vec<Check> {
    let mut checks = Vec::new();
    let location = suite::config_file_in(environ, home, Relative::Keep);
    let present = location.is_file();
    checks.push(Check {
        name: "config",
        ok: present,
        detail: if present {
            location.display().to_string()
        } else {
            format!("not found: {}", location.display())
        },
    });
    match launch_command(config, environ, home, &[]) {
        Ok(launch) => match launch.kind {
            Kind::Native => {
                checks.push(Check {
                    name: "python",
                    ok: true,
                    detail: "not needed: native Fennec binary".to_string(),
                });
                checks.push(Check {
                    name: "engine",
                    ok: true,
                    detail: format!("fennec {}", launch.argv[0]),
                });
            }
            Kind::Colibri => {
                checks.push(Check {
                    name: "python",
                    ok: true,
                    detail: launch.argv[0].clone(),
                });
                checks.push(Check {
                    name: "engine",
                    ok: true,
                    detail: format!("colibri {}", launch.argv[1]),
                });
            }
        },
        Err(error) => {
            checks.push(Check { name: "python", ok: false, detail: error.clone() });
            checks.push(Check { name: "engine", ok: false, detail: error });
        }
    }
    let model = suite::env_value(environ, "FENNEC_MODEL").or_else(|| config.get("model").and_then(Value::as_str));
    if let Some(model) = model {
        let path = suite::expand_in(model, home, Relative::Keep);
        checks.push(Check {
            name: "model",
            ok: path.is_dir(),
            detail: path.display().to_string(),
        });
    }
    let root = suite_root(config, environ, home);
    let found = SUITE_REPOS.iter().filter(|name| root.join(name).join(".git").is_dir()).count();
    checks.push(Check {
        name: "suite",
        ok: found > 0,
        detail: format!("{found} checkout(s) under {}", root.display()),
    });
    let pointer = home.join(".fennec").join("tokeneconomizer.path");
    let mut core = suite::env_value(environ, "FENNEC_TOKENECONOMIZER")
        .or_else(|| suite::env_value(environ, "TE_CORE_PATH"))
        .map(str::to_string);
    if core.is_none() && pointer.is_file() {
        core = std::fs::read_to_string(&pointer)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
    }
    let ready = core.as_ref().is_some_and(|value| {
        Path::new(value).join("tokeneconomizer").join("plugin").join("hook.py").is_file()
    });
    checks.push(Check {
        name: "tokeneconomizer",
        ok: ready,
        detail: core.unwrap_or_else(|| "not configured".to_string()),
    });
    checks
}

pub fn print_doctor(checks: &[Check]) -> i32 {
    for check in checks {
        println!("{} {:16} {}", if check.ok { "ok  " } else { "fail" }, check.name, check.detail);
    }
    i32::from(
        checks
            .iter()
            .filter(|check| matches!(check.name, "python" | "engine"))
            .any(|check| !check.ok),
    )
}

/// Candidate `fennec-core` checkouts, nearest first: `FENNEC_CORE`, the suite
/// root the config names, this binary's own directory and its parents, then the
/// working directory. The same places `core_module()` looked, without an
/// interpreter to import through.
pub fn core_candidates(
    config: &Map<String, Value>,
    environ: &Environ,
    home: &Path,
    exe_dir: &Path,
) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(value) = suite::env_nonblank(environ, "FENNEC_CORE") {
        paths.push(suite::expand_in(value, home, Relative::Keep));
    }
    paths.push(suite_root(config, environ, home).join("fennec-core"));
    let mut ancestor = Some(exe_dir.to_path_buf());
    while let Some(directory) = ancestor {
        let candidate = directory.join("fennec-core");
        if !paths.iter().any(|existing| existing == &candidate) {
            paths.push(candidate);
        }
        ancestor = directory.parent().map(Path::to_path_buf);
    }
    if let Ok(current) = std::env::current_dir() {
        let candidate = current.join("fennec-core");
        if !paths.iter().any(|existing| existing == &candidate) {
            paths.push(candidate);
        }
    }
    paths
}

/// A checkout that carries every module the verb needs: one `fennec_core`
/// package is the unit, so a half-populated checkout is not a target.
fn core_dir_for(paths: &[PathBuf], modules: &[&str]) -> Option<PathBuf> {
    paths.iter().find(|path| {
        modules.iter().all(|module| path.join("fennec_core").join(format!("{module}.py")).is_file())
    }).cloned()
}

fn not_found(paths: &[PathBuf], name: &str) -> String {
    let places =
        paths.iter().map(|path| path.display().to_string()).collect::<Vec<_>>().join(", ");
    format!("fennec {name} {NEEDS_CORE} (looked in {places})")
}

#[derive(Debug)]
pub struct CoreTarget {
    pub python: PathBuf,
    pub root: Option<PathBuf>,
    pub module: String,
}

/// Where `fennec_core.<module>` (and its companions) live for this run: a
/// checkout on the candidate list, or this interpreter's own environment when no
/// checkout carries it.
pub fn core_target(
    modules: &[&str],
    config: &Map<String, Value>,
    environ: &Environ,
    home: &Path,
    exe_dir: &Path,
    name: &str,
) -> Result<CoreTarget, String> {
    let python = python_for(config, environ, home);
    if !python_is_usable(&python) {
        return Err(python_missing(&python));
    }
    let paths = core_candidates(config, environ, home, exe_dir);
    let root = core_dir_for(&paths, modules);
    if root.is_none() {
        for module in modules {
            let full = format!("fennec_core.{module}");
            let probe = Command::new(&python)
                .args(["-c", PROBE, &full])
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            if !matches!(probe, Ok(status) if status.success()) {
                return Err(not_found(&paths, name));
            }
        }
    }
    let module = format!("fennec_core.{}", modules.first().unwrap_or(&""));
    Ok(CoreTarget { python, root, module })
}

/// Run the snippet with the module's home on `PYTHONPATH`, stdio inherited so
/// the wizard stays interactive and an ACP loop keeps its pipe.
pub fn run_core(target: &CoreTarget, snippet: &str, arguments: &[String]) -> i32 {
    let mut command = Command::new(&target.python);
    if let Some(root) = &target.root {
        command.env("PYTHONPATH", root);
    }
    command.arg("-c").arg(snippet).arg(&target.module).args(arguments);
    match command.status() {
        Ok(status) => status.code().unwrap_or(-1),
        Err(error) => {
            eprintln!(
                "fennec: cannot start {}: {error}. Set FENNEC_PYTHON or the python field in the \
                 Fennec configuration to a working interpreter.",
                target.python.display()
            );
            2
        }
    }
}

/// `fennec <engine arguments...>`: exec the engine with the config's pins. A
/// missing engine is exit 2, except for `--help`, which prints the launcher's
/// own usage instead so the command stays self-describing without a checkout.
pub fn dispatch_engine(
    config: &Map<String, Value>,
    environ: &Environ,
    home: &Path,
    arguments: &[String],
) -> i32 {
    let launch = match launch_command(config, environ, home, arguments) {
        Ok(launch) => launch,
        Err(error) => {
            if matches!(
                arguments.first().map(String::as_str),
                Some("--help") | Some("-h") | Some("help")
            ) {
                print!("{HELP}");
                return 0;
            }
            eprintln!("fennec: {error}");
            return 2;
        }
    };
    let mut command = Command::new(&launch.argv[0]);
    command.args(&launch.argv[1..]);
    for (key, value) in engine_pins(config, environ) {
        command.env(key, value);
    }
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    match command.status() {
        Ok(status) => status.code().unwrap_or(-1),
        Err(error) => {
            eprintln!("fennec: {error}");
            2
        }
    }
}

/// `git pull --ff-only` for [`update_suite`]: the exit code and both streams, so
/// a checkout that refuses to fast-forward reports what git said.
pub fn pull(argv: &[String]) -> (i32, String, String) {
    let mut command = Command::new(&argv[0]);
    git::quiet(&mut command);
    let output = command
        .args(&argv[1..])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output();
    match output {
        Ok(output) => (
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stdout).into_owned(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ),
        Err(error) => (127, String::new(), error.to_string()),
    }
}

/// The one error shape the launcher had: a `ValueError` on stderr as
/// `fennec: {message}` with exit 2, except for `--help`, which stays useful
/// without a checkout, engine or config file.
fn report(error: &str, verb: &str) -> i32 {
    if matches!(verb, "--help" | "-h" | "help") {
        print!("{HELP}");
        return 0;
    }
    eprintln!("fennec: {error}");
    2
}

fn no_extra(verb: &str) -> i32 {
    report(&format!("fennec {verb} takes no extra arguments"), "")
}

/// Run a `fennec_core` module, or report that this machine has no core to run.
fn core_call(
    modules: &[&str],
    snippet: &str,
    arguments: &[String],
    name: &str,
    config: &Map<String, Value>,
    environ: &Environ,
    home: &Path,
    exe_dir: &Path,
    verb: &str,
) -> i32 {
    match core_target(modules, config, environ, home, exe_dir, name) {
        Ok(target) => run_core(&target, snippet, arguments),
        Err(error) => report(&error, verb),
    }
}

/// `fennec acp` / `fennec acp --check`: the protocol stays in `fennec_core.acp`,
/// which is what an editor is pointed at. `--check` answers with what that agent
/// advertises, without starting the engine.
fn acp(rest: &[String], config: &Map<String, Value>, environ: &Environ, home: &Path, exe_dir: &Path) -> i32 {
    if rest.first().map(String::as_str) == Some("--check") {
        return core_call(&["acp"], ACP_CHECK, &[], "acp", config, environ, home, exe_dir, "");
    }
    if !rest.is_empty() {
        return report(&format!("unknown fennec acp argument: {}", rest[0]), "");
    }
    core_call(&["acp"], ACP_PIPE, &[], "acp", config, environ, home, exe_dir, "")
}

/// The launcher's own dispatch: the name the crate is started under decides
/// whether the process answers as `fennec`. `preflight` is handed back in as a
/// callback because the model budget is answered by the Rust preflight crate,
/// which lives with the glue rather than with the launcher.
pub fn dispatch(
    argv: &[String],
    version: &str,
    exe_dir: &Path,
    preflight: &dyn Fn(&[String], &Map<String, Value>) -> i32,
) -> i32 {
    let environ = suite::real_environ();
    let home = suite::home_in(&environ);
    let verb = argv.first().map(String::as_str).unwrap_or("");
    let rest: Vec<String> = argv.iter().skip(1).cloned().collect();
    if matches!(verb, "--version" | "-V") {
        println!("fennec {version}");
        return 0;
    }
    // The wizard is what writes the config file, so it runs before the strict
    // load that refuses an explicit FENNEC_CONFIG which does not exist yet.
    if verb == "setup" {
        return core_call(&["setup_wizard"], WIZARD, &rest, "setup", &Map::new(), &environ,
                         &home, exe_dir, verb);
    }
    let config = match load_config(&environ, &home) {
        Ok(config) => config,
        Err(error) => return report(&error, verb),
    };
    match verb {
        "update" => {
            if !rest.is_empty() {
                return no_extra(verb);
            }
            let outcome = update_suite(&config, &environ, &home, &pull);
            for (to_stderr, line) in &outcome.lines {
                if *to_stderr {
                    eprintln!("{line}");
                } else {
                    println!("{line}");
                }
            }
            outcome.code
        }
        "diagnose" => {
            if !rest.is_empty() {
                return no_extra(verb);
            }
            print_doctor(&doctor(&config, &environ, &home))
        }
        "preflight" => preflight(&rest, &config),
        "acp" => acp(&rest, &config, &environ, &home, exe_dir),
        "outlets" => {
            if !rest.is_empty() {
                return no_extra(verb);
            }
            core_call(&["config", "outlets"], OUTLETS_CHECK, &[], "outlets", &config,
                      &environ, &home, exe_dir, verb)
        }
        _ => dispatch_engine(&config, &environ, &home, argv),
    }
}
