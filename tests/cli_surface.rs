//! The launcher and the glue as a user meets them: two binaries started with an
//! environment, a stdout, a stderr and an exit code. Everything a test needs
//! from the machine is replaced by a sandbox home, so the assertions are about
//! this crate and not about whoever runs it.
//!
//! Verbs that hand the process to a `fennec_core` module, and the legacy
//! `c/coli` passthrough, need an interpreter. The Rust CI lane installs none,
//! so those tests skip themselves out loud rather than failing on a host that
//! has no Python - the same contract the engine lanes use for optional tools.

use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;

const FENNEC: &str = env!("CARGO_BIN_EXE_fennec");
const GLUE: &str = env!("CARGO_BIN_EXE_fennec-rs");

static SEQUENCE: AtomicU32 = AtomicU32::new(0);
static INTERPRETER: OnceLock<Option<String>> = OnceLock::new();

/// A Python this host can start, or none. Cached because discovering it costs a
/// spawn.
fn interpreter() -> Option<&'static str> {
    INTERPRETER.get_or_init(|| {
        for name in ["python", "python3"] {
            let started = Command::new(name).args(["-c", "print(1)"])
                .stdout(Stdio::null()).stderr(Stdio::null()).status();
            if matches!(started, Ok(status) if status.success()) {
                return Some(name.to_string());
            }
        }
        None
    })
    .as_deref()
}

fn skip_without_python(verb: &str) -> bool {
    if interpreter().is_some() {
        return false;
    }
    eprintln!("skipped: `fennec {verb}` needs a Python interpreter and this host has none");
    true
}

struct Sandbox(PathBuf);

impl Sandbox {
    fn new(tag: &str) -> Sandbox {
        let root = std::env::temp_dir().join(format!(
            "fennec-cli-surface-{tag}-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("a temporary sandbox can be created");
        Sandbox(root)
    }

    fn path(&self, relative: &str) -> PathBuf {
        let mut path = self.0.clone();
        for part in relative.split('/') {
            if !part.is_empty() {
                path.push(part);
            }
        }
        path
    }

    fn dir(&self, relative: &str) -> PathBuf {
        let path = self.path(relative);
        std::fs::create_dir_all(&path).expect("a sandbox directory can be created");
        path
    }

    fn file(&self, relative: &str, body: &str) -> PathBuf {
        let path = self.path(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("a sandbox directory can be created");
        }
        std::fs::write(&path, body).expect("a sandbox file can be written");
        path
    }

    /// A legacy Colibri checkout that reports what it was handed.
    fn stub_engine(&self) -> PathBuf {
        let home = self.dir("Engine");
        self.file("Engine/c/coli", r#"import json, os, sys
keys = ("FENNEC_MODEL", "COLI_MODEL", "FENNEC_VULKAN", "COLI_VULKAN",
        "HF_HOME", "PIP_CACHE_DIR", "FENNEC_HOME", "PYTHONPATH")
print(json.dumps({"argv": sys.argv[1:],
                  "env": {key: os.environ.get(key) for key in keys}}))
sys.exit(int(os.environ.get("FENNEC_STUB_EXIT", "3")))
"#);
        home
    }

    /// A `fennec_core` checkout whose modules answer instead of doing work.
    fn fake_core(&self) -> PathBuf {
        let root = self.dir("Suite/fennec-core/fennec_core");
        self.file("Suite/fennec-core/fennec_core/__init__.py", "");
        self.file("Suite/fennec-core/fennec_core/acp.py", r#"import json


def check():
    return {"name": "fake-agent", "transport": "stdio"}


def main(argv=None):
    print(json.dumps({"main": list(argv or [])}))
    return 0
"#);
        self.file("Suite/fennec-core/fennec_core/config.py", r#"def load_config():
    return {"ctx": 7}
"#);
        self.file("Suite/fennec-core/fennec_core/outlets.py", r#"class Outlets:
    def __init__(self, loaded):
        self.loaded = loaded

    def status(self):
        return {"ctx": self.loaded.get("ctx"), "jev": {"ready": False}}
"#);
        self.file("Suite/fennec-core/fennec_core/setup_wizard.py", r#"import json


def main(argv, config=None):
    print(json.dumps({"argv": list(argv), "config": config}))
    return 0
"#);
        root
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Only the variables a process needs to start at all, plus what the test asks
/// for. Nothing from the developer's own session leaks in, which is the whole
/// point: the real `~/.fennec/config.json` must not decide these assertions.
fn essentials() -> Vec<(String, String)> {
    let mut kept = Vec::new();
    for key in ["PATH", "SystemRoot", "windir", "ComSpec", "PATHEXT",
                "TEMP", "TMP", "TMPDIR", "LANG", "LC_ALL"] {
        if let Ok(value) = std::env::var(key) {
            kept.push((key.to_string(), value));
        }
    }
    kept
}

fn run(exe: &str, args: &[&str], extra: &[(String, String)]) -> Output {
    let mut command = Command::new(exe);
    command.args(args).env_clear().stdin(Stdio::null());
    for (key, value) in essentials() {
        command.env(key, value);
    }
    for (key, value) in extra {
        command.env(key, value);
    }
    command.output().expect("the built binary starts")
}

fn text(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn error(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn code(output: &Output) -> i32 {
    output.status.code().expect("the process exited with a code")
}

fn parsed(output: &Output) -> Value {
    serde_json::from_str(&text(output))
        .unwrap_or_else(|reason| panic!("not JSON: {reason}\n{}", text(output)))
}

/// A sandbox home that has no engine anywhere in it, which is the shape most of
/// these tests need: the launcher's own answers must not depend on a checkout.
fn no_engine(sandbox: &Sandbox) -> Vec<(String, String)> {
    let empty = sandbox.dir("Empty");
    vec![("FENNEC_HOME".to_string(), empty.display().to_string()),
         ("HOME".to_string(), sandbox.0.display().to_string()),
         ("USERPROFILE".to_string(), sandbox.0.display().to_string())]
}

fn with_engine(sandbox: &Sandbox, home: &Path, extra: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut vars: Vec<(String, String)> = vec![
        ("FENNEC_HOME".to_string(), home.display().to_string()),
        ("HOME".to_string(), sandbox.0.display().to_string()),
        ("USERPROFILE".to_string(), sandbox.0.display().to_string()),
    ];
    vars.extend(extra.iter().map(|(key, value)| (key.to_string(), value.to_string())));
    vars
}

// ------------------------------------------------------------------ own answers

#[test]
fn the_version_is_the_crate_version_with_or_without_an_engine() {
    let sandbox = Sandbox::new("version");
    let output = run(FENNEC, &["--version"], &no_engine(&sandbox));
    assert_eq!(code(&output), 0);
    assert_eq!(text(&output).trim(), format!("fennec {}", env!("CARGO_PKG_VERSION")));
    let output = run(GLUE, &["--version"], &no_engine(&sandbox));
    assert_eq!(text(&output).trim(), format!("fennec-rs {}", env!("CARGO_PKG_VERSION")));
}

#[test]
fn help_answers_from_the_launcher_when_there_is_no_engine() {
    let sandbox = Sandbox::new("help");
    for flag in ["--help", "-h", "help"] {
        let output = run(FENNEC, &[flag], &no_engine(&sandbox));
        assert_eq!(code(&output), 0, "{flag}: {}", error(&output));
        let stdout = text(&output);
        assert!(stdout.starts_with("fennec - launcher"), "{stdout}");
        assert!(stdout.contains("cargo build --release"), "{stdout}");
        assert!(stdout.contains("fennec preflight"), "{stdout}");
    }
    let output = run(GLUE, &["--help"], &no_engine(&sandbox));
    assert!(text(&output).contains("fennec-rs serve"), "{}", text(&output));
    assert_eq!(code(&output), 0);
    let bare = run(GLUE, &[], &no_engine(&sandbox));
    assert_eq!(code(&bare), 2, "no arguments is a usage answer, not a success");
}

#[test]
fn a_missing_engine_is_reported_once_and_exits_two() {
    let sandbox = Sandbox::new("no-engine");
    for arguments in [vec!["serve"], vec!["--model", "x"], vec!["wat"], vec!["run", "-p", "hi"]] {
        let argv: Vec<&str> = arguments.iter().copied().collect();
        let output = run(FENNEC, &argv, &no_engine(&sandbox));
        assert_eq!(code(&output), 2, "{argv:?}: {}", text(&output));
        let stderr = error(&output);
        assert!(stderr.starts_with("fennec: Fennec engine not found"), "{stderr}");
        assert!(stderr.contains("cargo build --release"), "{stderr}");
        assert!(text(&output).is_empty(), "the error belongs on stderr, not stdout");
    }
}

#[test]
fn an_unknown_command_on_the_glue_surface_is_two_with_the_usage() {
    let sandbox = Sandbox::new("glue-unknown");
    let output = run(GLUE, &["wat"], &no_engine(&sandbox));
    assert_eq!(code(&output), 2);
    let stderr = error(&output);
    assert!(stderr.contains("unknown command wat"), "{stderr}");
    assert!(stderr.contains("fennec-rs bench"), "{stderr}");
}

// --------------------------------------------------------------------- preflight

/// Two layers, two heads, four wide: 64 bytes of KV per token, once the two
/// bytes per element and the K and V halves are counted.
const GEOMETRY: &str = "{\"num_hidden_layers\": 2, \"num_attention_heads\": 2, \"head_dim\": 4}";

fn model_dir(sandbox: &Sandbox) -> PathBuf {
    let dir = sandbox.dir("models/tiny");
    sandbox.file("models/tiny/config.json", GEOMETRY);
    sandbox.file("models/tiny/weights.bin", &"x".repeat(2048));
    dir
}

#[test]
fn preflight_answers_from_the_rust_port_with_the_configured_budget() {
    let sandbox = Sandbox::new("preflight");
    let model = model_dir(&sandbox);
    let config = sandbox.file("config.json", "{\"ctx\": 100}");
    let vars = no_engine(&sandbox);
    let with_config: Vec<(String, String)> = {
        let mut all = vars.clone();
        all.push(("FENNEC_CONFIG".to_string(), config.display().to_string()));
        all
    };
    let pointer: Vec<&str> = ["preflight", "--model", model.to_str().unwrap(), "--json"].to_vec();
    let output = run(FENNEC, &pointer, &with_config);
    assert_eq!(code(&output), 0, "{}", error(&output));
    let report: Value = serde_json::from_str(&text(&output)).expect("preflight --json is JSON");
    // (2 K and V) x 2 layers x 2 heads x 4 elements per head x 2 bytes = 64.
    assert_eq!(report["kv_bytes_per_token"], json!(64));
    assert_eq!(report["ctx"], json!(100), "the shared config sets the budget");
    assert_eq!(report["kv_bytes"], json!(6400));
    assert_eq!(report["peak_bytes"], report["weights_bytes"].as_i64().unwrap() + 6400);
    assert_eq!(report["weights_bytes"], json!(2048 + GEOMETRY.len()));
    assert!(["vram", "hybrid", "disk"].contains(&report["weights_fit"].as_str().unwrap()));

    let explicit: Vec<&str> = ["preflight", "--model", model.to_str().unwrap(), "--ctx", "8", "--json"].to_vec();
    let output = run(FENNEC, &explicit, &with_config);
    let report: Value = serde_json::from_str(&text(&output)).expect("preflight --json is JSON");
    assert_eq!(report["ctx"], json!(8), "--ctx beats the configured budget");
    assert_eq!(report["kv_bytes"], json!(512));

    // The human report says the same thing in words.
    let human: Vec<&str> = ["preflight", "--model", model.to_str().unwrap()].to_vec();
    let output = run(FENNEC, &human, &with_config);
    let stdout = text(&output);
    assert!(stdout.contains("context      100 tokens"), "{stdout}");
    assert!(stdout.contains("KV at ctx"), "{stdout}");
    assert!(stdout.contains("weights fit"), "{stdout}");
    assert!(stdout.contains(&model.display().to_string()), "{stdout}");
}

#[test]
fn both_surfaces_answer_the_same_preflight() {
    let sandbox = Sandbox::new("preflight-parity");
    let model = model_dir(&sandbox);
    let launcher_args: Vec<&str> =
        ["preflight", "--model", model.to_str().unwrap(), "--ctx", "16", "--json"].to_vec();
    let launcher = run(FENNEC, &launcher_args, &no_engine(&sandbox));
    let glue_args: Vec<&str> =
        ["preflight", model.to_str().unwrap(), "--ctx", "16", "--json"].to_vec();
    let glue = run(GLUE, &glue_args, &no_engine(&sandbox));
    assert_eq!(code(&launcher), 0);
    assert_eq!(code(&glue), 0);
    let launcher: Value = serde_json::from_str(&text(&launcher)).expect("launcher json");
    let glue: Value = serde_json::from_str(&text(&glue)).expect("glue json");
    // The report's own numbers, not the live device probes: available RAM and
    // free disk are read per process, so they drift between two runs.
    for key in ["model", "ctx", "weights_bytes", "kv_bytes_per_token", "kv_bytes", "peak_bytes"] {
        assert_eq!(launcher[key], glue[key], "{key} differs between the two surfaces");
    }
    assert_eq!(launcher["notes"], glue["notes"]);
}

#[test]
fn preflight_needs_a_model_directory() {
    let sandbox = Sandbox::new("preflight-none");
    let argv: Vec<&str> = vec!["preflight", "--json"];
    let output = run(FENNEC, &argv, &no_engine(&sandbox));
    assert_eq!(code(&output), 2);
    assert!(error(&output).contains("needs a model directory"), "{}", error(&output));
}

// ------------------------------------------------------------------ config errors

#[test]
fn an_explicit_config_that_does_not_exist_is_refused_except_by_the_wizard() {
    let sandbox = Sandbox::new("config-missing");
    let missing = sandbox.path("nowhere.json");
    let mut vars = no_engine(&sandbox);
    vars.push(("FENNEC_CONFIG".to_string(), missing.display().to_string()));
    let argv: Vec<&str> = vec!["diagnose"];
    let output = run(FENNEC, &argv, &vars);
    assert_eq!(code(&output), 2);
    assert!(error(&output).starts_with("fennec: configured file does not exist:"), "{}", error(&output));

    // The wizard is what writes that file, so it must still run.
    let argv: Vec<&str> = vec!["setup", "--yes"];
    let output = run(FENNEC, &argv, &vars);
    assert!(!error(&output).contains("does not exist"), "{}", error(&output));
    assert_ne!(code(&output), 0, "there is no core to run here either");

    let argv: Vec<&str> = vec!["--help"];
    let output = run(FENNEC, &argv, &vars);
    assert_eq!(code(&output), 0, "help stays useful without a config file");
}

#[test]
fn a_config_that_is_not_json_is_a_message_and_not_a_traceback() {
    let sandbox = Sandbox::new("config-broken");
    for (tag, body) in [("array", "[1, 2]"), ("broken", "{engine_root:")] {
        let path = sandbox.file(&format!("{tag}.json"), body);
        let mut vars = no_engine(&sandbox);
        vars.push(("FENNEC_CONFIG".to_string(), path.display().to_string()));
        let argv: Vec<&str> = vec!["update"];
        let output = run(FENNEC, &argv, &vars);
        assert_eq!(code(&output), 2, "{tag}");
        let stderr = error(&output);
        assert!(stderr.starts_with("fennec: configuration is not valid JSON:")
            || stderr.starts_with("fennec: configuration must be a JSON object:"), "{stderr}");
        assert!(!stderr.contains("Traceback"), "{stderr}");
    }
}

// ---------------------------------------------------------------------- verbs

#[test]
fn suite_verbs_answer_without_an_engine_and_refuse_extra_arguments() {
    let sandbox = Sandbox::new("verbs");
    let vars = no_engine(&sandbox);

    // diagnose is read-only: it runs where there is no engine at all.
    let argv: Vec<&str> = vec!["diagnose"];
    let output = run(FENNEC, &argv, &vars);
    let stdout = text(&output);
    for name in ["config", "python", "engine", "suite", "tokeneconomizer"] {
        assert!(stdout.contains(name), "{stdout}");
    }
    assert!(stdout.contains("fail") && stdout.contains("engine"), "{stdout}");
    assert_eq!(code(&output), 1, "no engine is a failing report, not a crash");

    for (verb, extra) in [("diagnose", "--all"), ("update", "--force"), ("outlets", "--json")] {
        let argv: Vec<&str> = vec![verb, extra];
        let output = run(FENNEC, &argv, &vars);
        assert_eq!(code(&output), 2, "{verb} {extra}");
        assert!(error(&output).contains("takes no extra arguments"), "{}", error(&output));
    }

    // An acp argument the module does not take is refused, not passed on.
    let argv: Vec<&str> = vec!["acp", "--terminal"];
    let output = run(FENNEC, &argv, &vars);
    assert_eq!(code(&output), 2);
    assert!(error(&output).contains("unknown fennec acp argument: --terminal"), "{}", error(&output));
}

#[test]
fn the_core_verbs_reach_a_fennec_core_checkout_and_report_what_it_says() {
    if skip_without_python("acp") {
        return;
    }
    let sandbox = Sandbox::new("core-verbs");
    sandbox.fake_core();
    let suite = sandbox.path("Suite");
    let mut vars = with_engine(&sandbox, &sandbox.dir("Engine"),
                               &[("FENNEC_PYTHON", interpreter().unwrap())]);
    vars.push(("FENNEC_SUITE".to_string(), suite.display().to_string()));

    let argv: Vec<&str> = vec!["acp", "--check"];
    let output = run(FENNEC, &argv, &vars);
    assert_eq!(code(&output), 0, "{}", error(&output));
    let advertised: Value = serde_json::from_str(&text(&output)).expect("acp --check prints JSON");
    assert_eq!(advertised["name"], json!("fake-agent"));
    assert_eq!(advertised["transport"], json!("stdio"));

    let argv: Vec<&str> = vec!["acp"];
    let output = run(FENNEC, &argv, &vars);
    assert_eq!(code(&output), 0, "{}", error(&output));
    assert_eq!(parsed(&output)["main"], json!([]), "the pipe is started with no argv of its own");

    let argv: Vec<&str> = vec!["outlets"];
    let output = run(FENNEC, &argv, &vars);
    assert_eq!(code(&output), 0, "{}", error(&output));
    let status = parsed(&output);
    assert_eq!(status["ctx"], json!(7), "the config module's own loader was used");
    assert_eq!(status["jev"]["ready"], json!(false));

    let argv: Vec<&str> = vec!["setup", "--yes"];
    let output = run(FENNEC, &argv, &vars);
    assert_eq!(code(&output), 0, "{}", error(&output));
    assert_eq!(parsed(&output)["argv"], json!(["--yes"]));
    assert_eq!(parsed(&output)["config"], json!({}));
}

#[test]
fn a_module_exit_code_comes_back_through_the_handover() {
    if skip_without_python("outlets") {
        return;
    }
    let sandbox = Sandbox::new("core-exit");
    sandbox.fake_core();
    // The suite's own outlets module reporting "not ready" is a 2 in
    // fennec-core; the launcher is a pipe here, not a filter.
    sandbox.file("Suite/fennec-core/fennec_core/outlets.py", r#"class Outlets:
    def __init__(self, loaded):
        self.loaded = loaded

    def status(self):
        raise SystemExit(4)
"#);
    let mut vars = with_engine(&sandbox, &sandbox.dir("Engine"),
                               &[("FENNEC_PYTHON", interpreter().unwrap())]);
    vars.push(("FENNEC_SUITE".to_string(), sandbox.path("Suite").display().to_string()));
    let argv: Vec<&str> = vec!["outlets"];
    let output = run(FENNEC, &argv, &vars);
    assert_eq!(code(&output), 4, "{} / {}", text(&output), error(&output));
}

// ------------------------------------------------------------- engine passthrough

#[test]
fn the_engine_gets_the_arguments_and_its_exit_code_comes_back() {
    if skip_without_python("the engine passthrough") {
        return;
    }
    let sandbox = Sandbox::new("passthrough");
    let home = sandbox.stub_engine();
    let mut vars = with_engine(&sandbox, &home, &[("FENNEC_PYTHON", interpreter().unwrap())]);
    vars.push(("FENNEC_STUB_EXIT".to_string(), "3".to_string()));
    let argv: Vec<&str> = vec!["serve", "--port", "8787", "--device", "vulkan"];
    let output = run(FENNEC, &argv, &vars);
    assert_eq!(code(&output), 3, "the child's code is the launcher's code");
    let report = parsed(&output);
    assert_eq!(report["argv"], json!(["serve", "--port", "8787", "--device", "vulkan"]));
    assert_eq!(report["env"]["FENNEC_HOME"], json!(home.display().to_string()));
    assert!(error(&output).is_empty(), "{}", error(&output));

}

#[test]
fn the_configured_model_backend_and_cache_reach_the_legacy_child_under_both_spellings() {
    if skip_without_python("the environment pins") {
        return;
    }
    let sandbox = Sandbox::new("pins");
    let home = sandbox.stub_engine();
    let model = sandbox.dir("models/qwen");
    let cache = sandbox.dir("cache");
    let config = sandbox.file("config.json", &json!({
        "model": model, "backend": "vulkan", "cache_root": cache
    }).to_string());
    let mut vars = with_engine(&sandbox, &home, &[("FENNEC_PYTHON", interpreter().unwrap())]);
    vars.push(("FENNEC_CONFIG".to_string(), config.display().to_string()));
    let argv: Vec<&str> = vec!["run"];
    let report = parsed(&run(FENNEC, &argv, &vars));
    assert_eq!(report["env"]["FENNEC_MODEL"], json!(model.display().to_string()));
    assert_eq!(report["env"]["COLI_MODEL"], json!(model.display().to_string()));
    assert_eq!(report["env"]["FENNEC_VULKAN"], json!("1"));
    assert_eq!(report["env"]["COLI_VULKAN"], json!("1"));
    assert_eq!(report["env"]["HF_HOME"], json!(cache.join("huggingface").display().to_string()));
    assert_eq!(report["env"]["PIP_CACHE_DIR"], json!(cache.join("pip").display().to_string()));

    // An export wins, and the launcher never rewrites the name it was set on.
    let mut exported = vars.clone();
    exported.push(("FENNEC_MODEL".to_string(), "exported/model".to_string()));
    exported.push(("FENNEC_VULKAN".to_string(), "0".to_string()));
    let argv: Vec<&str> = vec!["run"];
    let report = parsed(&run(FENNEC, &argv, &exported));
    assert_eq!(report["env"]["FENNEC_MODEL"], json!("exported/model"));
    assert_eq!(report["env"]["FENNEC_VULKAN"], json!("0"));

    // A backend the engine names directly is passed through, not Vulkan-flagged.
    let cuda = sandbox.file("cuda.json", &json!({"backend": "cuda"}).to_string());
    let argv: Vec<&str> = vec!["run"];
    let mut cuda_vars = with_engine(&sandbox, &home, &[("FENNEC_PYTHON", interpreter().unwrap())]);
    cuda_vars.push(("FENNEC_CONFIG".to_string(), cuda.display().to_string()));
    let report = parsed(&run(FENNEC, &argv, &cuda_vars));
    assert_eq!(report["env"]["FENNEC_VULKAN"], Value::Null);
}

// ------------------------------------------------------------------ glue bridge

#[test]
fn the_glue_surface_serves_the_bridge_the_core_spawns() {
    let sandbox = Sandbox::new("bridge");
    let mut command = Command::new(GLUE);
    command.arg("serve").env_clear().stdin(Stdio::piped()).stdout(Stdio::piped());
    for (key, value) in essentials() {
        command.env(key, value);
    }
    command.env("HOME", &sandbox.0);
    let mut child = command.spawn().expect("the bridge starts");
    child.stdin.as_mut().unwrap()
        .write_all(b"{\"op\":\"ping\"}\n{\"op\":\"urlgate\",\"url\":\"http://127.0.0.1:9/\"}\n{\"op\":\"exit\"}\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(code(&output), 0, "{}", error(&output));
    let frames = text(&output);
    assert!(frames.contains("fennec-rs"), "{frames}");
    assert!(frames.contains("\"ok\":true") || frames.contains("\"ok\": true"), "{frames}");
    assert!(frames.contains("blocked"), "the urlgate answer is in the stream: {frames}");
    assert!(frames.trim_end().ends_with('}'), "{frames}");
}

#[test]
fn the_launcher_surface_does_not_answer_glue_verbs_as_its_own() {
    let sandbox = Sandbox::new("surface");
    // `compact` belongs to the glue: under the launcher name it is an engine
    // argument, so with no engine the answer is the engine remedy, not a port.
    let argv: Vec<&str> = vec!["compact", "transcript.json"];
    let output = run(FENNEC, &argv, &no_engine(&sandbox));
    assert_eq!(code(&output), 2);
    assert!(error(&output).contains("Fennec engine not found"), "{}", error(&output));
    let argv: Vec<&str> = vec!["compact"];
    let output = run(GLUE, &argv, &no_engine(&sandbox));
    assert_eq!(code(&output), 2);
    assert!(error(&output).contains("compact"), "{}", error(&output));
}
