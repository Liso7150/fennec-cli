//! The launcher's decisions, tested as functions: each one takes the
//! environment, the home and the config as arguments, so a test can describe a
//! machine - engine built or not, suite half installed - and
//! read the answer without touching this host. These tests are what made the
//! Python launcher's `tests/test_cli.py` portable: same probe order, same pins,
//! same report shapes, with the interpreter and the filesystem replaced by a
//! sandbox instead of by monkeypatching.

use fennec_client::suite::Environ;
use fennec_rs::launcher::{self, Check};
use serde_json::{json, Map};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

static SEQUENCE: AtomicU32 = AtomicU32::new(0);

fn binary_name() -> &'static str {
    if cfg!(windows) { "fennec.exe" } else { "fennec" }
}

/// A throwaway home, engine and suite, removed when the test ends.
struct Sandbox(PathBuf);

impl Sandbox {
    fn new(tag: &str) -> Sandbox {
        let root = std::env::temp_dir().join(format!(
            "fennec-cli-{tag}-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("a temporary sandbox can be created");
        Sandbox(root)
    }

    fn path(&self, relative: &str) -> PathBuf {
        self.0.join(relative)
    }

    fn dir(&self, relative: &str) -> PathBuf {
        let path = self.0.join(relative);
        std::fs::create_dir_all(&path).expect("a sandbox directory can be created");
        path
    }

    fn file(&self, relative: &str, body: &str) -> PathBuf {
        write(&self.0.join(relative), body)
    }

    /// A native engine binary at one of the relative dirs the probe walks.
    fn engine_at(&self, home: &Path, relative: &str) -> PathBuf {
        write(&home.join(relative).join(binary_name()), "")
    }

    fn engine(&self) -> PathBuf {
        let home = self.dir("Engine");
        self.engine_at(&home, "target/release");
        home
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write(path: &Path, body: &str) -> PathBuf {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("a sandbox directory can be created");
    }
    std::fs::write(path, body).expect("a sandbox file can be written");
    path.to_path_buf()
}

/// Environment variables, the way the launcher reads them. `vars!` rather than
/// `env!` because the standard macro of that name is still needed elsewhere.
macro_rules! vars {
    ($($key:expr => $value:expr),* $(,)?) => {{
        #[allow(unused_mut)]
        let mut environ = Environ::new();
        $(environ.insert($key.to_string(), $value.to_string());)*
        environ
    }};
}

macro_rules! conf {
    ($($key:expr => $value:expr),* $(,)?) => {{
        #[allow(unused_mut)]
        let mut map = Map::new();
        $(map.insert($key.to_string(), json!($value));)*
        map
    }};
}

fn pointing_at(home: &Path) -> Environ {
    vars!["FENNEC_HOME" => home.display().to_string()]
}

/// What the launcher reports for a path it found: canonicalised, so a test
/// compares against the spelling it gets back rather than the one it wrote.
fn real(path: &Path) -> PathBuf {
    launcher::canonical(path)
}

fn stdout_lines(lines: &[(bool, String)]) -> Vec<String> {
    lines.iter().filter(|(to_stderr, _)| !*to_stderr).map(|(_, line)| line.clone()).collect()
}

fn stderr_lines(lines: &[(bool, String)]) -> Vec<String> {
    lines.iter().filter(|(to_stderr, _)| *to_stderr).map(|(_, line)| line.clone()).collect()
}

/// A git runner that records the argv it was handed. `update_suite` takes it as
/// an argument precisely so a suite update can be exercised with no checkout
/// and no network.
fn recorder(answer: impl Fn(&[String]) -> (i32, String, String) + 'static)
    -> (Rc<RefCell<Vec<Vec<String>>>>, impl Fn(&[String]) -> (i32, String, String)) {
    let calls = Rc::new(RefCell::new(Vec::new()));
    let shared = calls.clone();
    let runner = move |argv: &[String]| {
        shared.borrow_mut().push(argv.to_vec());
        answer(argv)
    };
    (calls, runner)
}

// ---------------------------------------------------------------- engine probe

#[test]
fn the_release_binary_is_probed_before_the_debug_one() {
    let sandbox = Sandbox::new("release-first");
    let home = sandbox.dir("Engine");
    sandbox.engine_at(&home, "target/debug");
    sandbox.engine_at(&home, ".");
    let release = sandbox.engine_at(&home, "target/release");
    let engine = launcher::engine_target(&conf!(), &pointing_at(&home), &home).unwrap();
    assert_eq!(engine.entry, real(&release));
    assert_eq!(engine.home, real(&home));
}

#[test]
fn the_root_binary_is_used_when_nothing_is_in_target() {
    let sandbox = Sandbox::new("root-binary");
    let home = sandbox.dir("Engine");
    let at_root = sandbox.engine_at(&home, ".");
    let engine = launcher::engine_target(&conf!(), &pointing_at(&home), &home).unwrap();
    assert_eq!(engine.entry, real(&at_root));
    assert_eq!(engine.entry.parent().unwrap(), real(&home));
}

#[test]
fn a_debug_only_build_is_found_and_is_the_debug_one() {
    let sandbox = Sandbox::new("debug-only");
    let home = sandbox.dir("Engine");
    let debug = sandbox.engine_at(&home, "target/debug");
    let engine = launcher::engine_target(&conf!(), &pointing_at(&home), &home).unwrap();
    assert_eq!(engine.entry, real(&debug));
}

#[test]
fn an_unbuilt_tree_reports_the_build_remedy() {
    let sandbox = Sandbox::new("unbuilt-tree");
    let home = sandbox.dir("Engine");
    sandbox.file("Engine/c/coli", "");
    let error = launcher::engine_target(&conf!(), &pointing_at(&home), &home).unwrap_err();
    assert!(error.contains("cargo build --release"), "{error}");
    assert!(error.contains("target/release/fennec"), "{error}");
}

#[test]
fn a_built_engine_is_found_in_a_tree_with_unrelated_files() {
    let sandbox = Sandbox::new("native-in-tree");
    let home = sandbox.dir("Engine");
    sandbox.file("Engine/c/coli", "");
    sandbox.engine_at(&home, "target/debug");
    let engine = launcher::engine_target(&conf!(), &pointing_at(&home), &home).unwrap();
    assert_eq!(engine.home, real(&home));
}

#[test]
fn an_exported_engine_home_is_the_only_place_probed() {
    let sandbox = Sandbox::new("exported-home");
    let exported = sandbox.dir("Exported");
    let configured = sandbox.dir("Configured");
    sandbox.engine_at(&configured, "target/release");
    let release = sandbox.engine_at(&exported, "target/release");
    let engine = launcher::engine_target(
        &conf!["engine_root" => configured.display().to_string()],
        &vars!["FENNEC_HOME" => exported.display().to_string()],
        &sandbox.0,
    )
    .unwrap();
    assert_eq!(engine.home, real(&exported));
    assert_eq!(engine.entry, real(&release));
}

#[test]
fn an_empty_exported_engine_home_falls_through_to_the_config() {
    let sandbox = Sandbox::new("empty-exported");
    let configured = sandbox.dir("Configured");
    sandbox.engine_at(&configured, "target/release");
    let engine = launcher::engine_target(
        &conf!["engine_root" => configured.display().to_string()],
        &vars!["FENNEC_HOME" => ""],
        &sandbox.0,
    )
    .unwrap();
    assert_eq!(engine.home, real(&configured));
}

#[test]
fn an_unconfigured_home_looks_only_in_fennec() {
    let sandbox = Sandbox::new("default-homes");
    sandbox.dir("Colibri");
    sandbox.file("Colibri/c/coli", "");
    assert!(launcher::engine_target(&conf!(), &vars!(), &sandbox.0).is_err());
    sandbox.engine_at(&sandbox.dir("Fennec"), "target/release");
    let engine = launcher::engine_target(&conf!(), &vars!(), &sandbox.0).unwrap();
    assert_eq!(engine.home, real(&sandbox.path("Fennec")));
}

#[test]
fn a_home_with_nothing_in_it_names_the_remedy() {
    let sandbox = Sandbox::new("no-engine");
    let empty = sandbox.dir("Empty");
    let error = launcher::engine_target(&conf!(), &vars!["FENNEC_HOME" => empty.display().to_string()],
                                        &sandbox.0)
        .unwrap_err();
    assert!(error.contains("cargo build --release"), "{error}");
    assert!(error.contains("Fennec engine not found"), "{error}");
    assert!(error.contains("installer"), "{error}");
}

// --------------------------------------------------------------------- launch

#[test]
fn a_native_launch_hands_the_binary_the_engine_arguments() {
    let sandbox = Sandbox::new("native-launch");
    let home = sandbox.dir("Engine");
    let binary = sandbox.engine_at(&home, "target/release");
    let arguments: Vec<String> = ["serve", "--port", "8787"].iter().map(|part| part.to_string()).collect();
    let launch = launcher::launch_command(&conf!(), &vars!["FENNEC_HOME" => home.display().to_string()],
                                          &sandbox.0, &arguments)
        .unwrap();
    assert_eq!(launch.argv[0], real(&binary).display().to_string());
    assert_eq!(&launch.argv[1..], arguments.as_slice());
}

#[test]
fn a_launch_without_a_built_engine_names_the_remedy() {
    let sandbox = Sandbox::new("launch-no-engine");
    let home = sandbox.dir("Engine");
    sandbox.file("Engine/c/coli", "");
    let error = launcher::launch_command(&conf!(),
                                         &vars!["FENNEC_HOME" => home.display().to_string()],
                                         &sandbox.0, &[])
        .unwrap_err();
    assert!(error.contains("cargo build --release"), "{error}");
}

#[test]
fn the_exported_interpreter_beats_the_configured_one() {
    let sandbox = Sandbox::new("python-order");
    let exported = sandbox.file("bin/exported-python", "");
    let configured = sandbox.file("bin/configured-python", "");
    let configured_value = configured.display().to_string();
    let from_config = conf!["python" => configured_value.clone()];
    assert_eq!(launcher::python_for(&from_config,
                                    &vars!["FENNEC_PYTHON" => exported.display().to_string()],
                                    &sandbox.0),
               real(&exported));
    assert_eq!(launcher::python_for(&from_config, &vars!(), &sandbox.0), real(&configured));
}

#[test]
fn an_interpreter_that_exists_is_usable_and_one_that_does_not_is_not() {
    let sandbox = Sandbox::new("python-usable");
    let present = sandbox.file("bin/python", "");
    assert!(launcher::python_is_usable(&present));
    assert!(!launcher::python_is_usable(&sandbox.path("bin/absent")));
}

// ----------------------------------------------------------------------- pins

#[test]
fn a_configured_model_is_pinned_under_its_one_name() {
    let pins = launcher::engine_pins(&conf!["model" => "~/models/qwen"], &vars!());
    assert_eq!(pins, vec![("FENNEC_MODEL".to_string(), "~/models/qwen".to_string())]);
}

#[test]
fn an_exported_model_is_never_overwritten_and_only_the_gap_is_filled() {
    let pins = launcher::engine_pins(&conf!["model" => "configured"], &vars!["FENNEC_MODEL" => "exported"]);
    assert!(pins.is_empty(), "{pins:?}");
    // The legacy COLI_ spelling is not read or pinned: the native binary
    // speaks FENNEC_* only.
    let pins = launcher::engine_pins(&conf!["model" => "configured"], &vars!["COLI_MODEL" => "legacy"]);
    assert_eq!(pins, vec![("FENNEC_MODEL".to_string(), "configured".to_string())]);
}

#[test]
fn an_empty_exported_model_is_not_read_as_a_model_but_owns_its_name() {
    let pins = launcher::engine_pins(&conf!["model" => "configured"], &vars!["FENNEC_MODEL" => ""]);
    assert!(pins.is_empty(), "{pins:?}");
}

#[test]
fn the_vulkan_flag_is_pinned_for_the_backends_that_need_it() {
    let pins = launcher::engine_pins(&conf!["backend" => "vulkan"], &vars!());
    assert_eq!(pins, vec![("FENNEC_VULKAN".to_string(), "1".to_string())]);
    let pins = launcher::engine_pins(&conf!["backend" => "cpu"], &vars!());
    assert_eq!(pins, vec![("FENNEC_VULKAN".to_string(), "0".to_string())]);
}

#[test]
fn a_backend_the_engine_names_directly_is_not_pinned() {
    let pins = launcher::engine_pins(&conf!["backend" => "cuda"], &vars!());
    assert!(!pins.iter().any(|(key, _)| key.contains("VULKAN")), "{pins:?}");
    assert!(pins.is_empty(), "{pins:?}");
}

#[test]
fn a_shell_pin_on_the_name_the_child_reads_is_not_re_pinned() {
    let pins = launcher::engine_pins(&conf!["backend" => "vulkan"], &vars!["FENNEC_VULKAN" => "1"]);
    assert!(!pins.iter().any(|(key, _)| key.contains("VULKAN")), "{pins:?}");
}

#[test]
fn the_cache_root_becomes_the_hugging_face_and_pip_caches() {
    let pins = launcher::engine_pins(&conf!["cache_root" => "D:/cache"], &vars!());
    let cache = PathBuf::from("D:/cache");
    assert!(pins.contains(&("HF_HOME".to_string(), cache.join("huggingface").display().to_string())));
    assert!(pins.contains(&("PIP_CACHE_DIR".to_string(), cache.join("pip").display().to_string())));
    let pins = launcher::engine_pins(&conf!["cache_root" => "configured"], &vars!["FENNEC_CACHE" => "exported"]);
    assert!(pins.contains(&("HF_HOME".to_string(),
                            PathBuf::from("exported").join("huggingface").display().to_string())));
    let pins = launcher::engine_pins(&conf!["cache_root" => "D:/cache"], &vars!["PIP_CACHE_DIR" => "already"]);
    assert!(!pins.iter().any(|(key, _)| key == "PIP_CACHE_DIR"), "{pins:?}");
    assert!(pins.iter().any(|(key, _)| key == "HF_HOME"), "{pins:?}");
}

#[test]
fn a_config_with_no_cache_or_backend_pins_only_what_it_knows() {
    assert_eq!(launcher::engine_pins(&conf!["model" => "m"], &vars!()).len(), 1);
    assert!(launcher::engine_pins(&conf!(), &vars!()).is_empty());
}

// ---------------------------------------------------------------------- config

#[test]
fn an_absent_default_config_is_empty_rather_than_an_error() {
    let sandbox = Sandbox::new("config-absent");
    assert!(launcher::load_config(&vars!(), &sandbox.0).unwrap().is_empty());
}

#[test]
fn an_explicit_config_that_does_not_exist_is_an_error() {
    let sandbox = Sandbox::new("config-missing");
    let missing = sandbox.path("nowhere.json");
    let error = launcher::load_config(&vars!["FENNEC_CONFIG" => missing.display().to_string()], &sandbox.0)
        .unwrap_err();
    assert!(error.starts_with("configured file does not exist:"), "{error}");
    assert!(error.contains("nowhere.json"), "{error}");
}

#[test]
fn a_config_that_is_not_an_object_is_refused_by_name() {
    let sandbox = Sandbox::new("config-array");
    let path = sandbox.file("config.json", "[1, 2]");
    let error = launcher::load_config(&vars!["FENNEC_CONFIG" => path.display().to_string()], &sandbox.0)
        .unwrap_err();
    assert!(error.starts_with("configuration must be a JSON object:"), "{error}");
}

#[test]
fn a_config_that_is_not_json_is_reported_without_a_traceback() {
    let sandbox = Sandbox::new("config-broken");
    let path = sandbox.file("config.json", "{engine_root:");
    let error = launcher::load_config(&vars!["FENNEC_CONFIG" => path.display().to_string()], &sandbox.0)
        .unwrap_err();
    assert!(error.starts_with("configuration is not valid JSON:"), "{error}");
}

#[test]
fn a_config_object_is_read_as_written() {
    let sandbox = Sandbox::new("config-object");
    let path = sandbox.file("config.json", "{\"engine_root\": \"D:/Colibri\", \"ctx\": 32768}");
    let config = launcher::load_config(&vars!["FENNEC_CONFIG" => path.display().to_string()], &sandbox.0)
        .unwrap();
    assert_eq!(config["engine_root"], json!("D:/Colibri"));
    assert_eq!(config["ctx"], json!(32768));
}

#[test]
fn the_suite_root_is_exported_then_configured_then_home() {
    let sandbox = Sandbox::new("suite-root");
    let config = conf!["suite_root" => "~/configured-suite"];
    assert_eq!(launcher::suite_root(&config, &vars!["FENNEC_SUITE" => "~/exported-suite"], &sandbox.0),
               sandbox.path("exported-suite"));
    assert_eq!(launcher::suite_root(&config, &vars!(), &sandbox.0), sandbox.path("configured-suite"));
    assert_eq!(launcher::suite_root(&conf!(), &vars!(), &sandbox.0), sandbox.path("Fennec-Suite"));
}

// ---------------------------------------------------------------------- update

#[test]
fn update_pulls_each_installed_checkout_and_nothing_else() {
    let sandbox = Sandbox::new("update-pulls");
    let root = sandbox.dir("Suite");
    sandbox.dir("Suite/fennec-core/.git");
    sandbox.dir("Suite/fennec-cli/.git");
    sandbox.dir("Suite/fennec-jetbrains");
    sandbox.dir("Suite/not-a-suite-repo/.git");
    let environ = vars!["FENNEC_SUITE" => root.display().to_string()];
    let (calls, runner) = recorder(|_| (0, "Already up to date.\n".to_string(), String::new()));
    let outcome = launcher::update_suite(&conf!(), &environ, &sandbox.0, &runner);
    assert_eq!(outcome.code, 0);
    let pulled = calls.borrow().clone();
    assert_eq!(pulled.len(), 2, "{pulled:?}");
    let expected: Vec<String> =
        ["git", "-C", root.join("fennec-core").to_str().unwrap(), "pull", "--ff-only"]
            .iter().map(|part| part.to_string()).collect();
    assert_eq!(pulled[0], expected);
    let second = pulled[1].clone();
    assert!(second.iter().any(|part| part.ends_with("fennec-cli")), "{second:?}");
    assert!(!pulled.iter().any(|argv| argv.iter().any(|part| part.contains("not-a-suite-repo"))));
    assert!(!pulled.iter().any(|argv| argv.iter().any(|part| part.contains("fennec-jetbrains"))));
    let report = stdout_lines(&outcome.lines);
    assert!(report.contains(&"Updated 2 checkout(s).".to_string()), "{report:?}");
    assert!(report.last().unwrap().contains("JustVugg/colibri"), "{report:?}");
    assert!(report.last().unwrap().contains("lyogavin/airllm"), "{report:?}");
    assert!(stderr_lines(&outcome.lines).is_empty());
}

#[test]
fn update_reports_what_git_said_and_exits_one_without_fixing_anything() {
    let sandbox = Sandbox::new("update-dirty");
    let root = sandbox.dir("Suite");
    sandbox.dir("Suite/fennec-core/.git");
    sandbox.dir("Suite/fennec-cli/.git");
    let environ = vars!["FENNEC_SUITE" => root.display().to_string()];
    let (_calls, runner) = recorder(|argv: &[String]| {
        if argv.iter().any(|part| part.ends_with("fennec-cli")) {
            (0, "Updating abc123..def456\nFast-forward\nsecond line\n".to_string(), String::new())
        } else {
            (1, String::new(),
             "error: Your local changes would be overwritten by merge\n".to_string())
        }
    });
    let outcome = launcher::update_suite(&conf!(), &environ, &sandbox.0, &runner);
    assert_eq!(outcome.code, 1);
    let report = stdout_lines(&outcome.lines);
    assert!(report.iter().any(|line| line.starts_with("fail fennec-core:")
        && line.contains("local changes")), "{report:?}");
    assert!(report.iter().any(|line| line == "ok   fennec-cli: second line"), "{report:?}");
    let failures = stderr_lines(&outcome.lines);
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert!(failures[0].contains("could not update fennec-core"), "{failures:?}");
    assert!(failures[0].contains("commit or stash"), "{failures:?}");
}

#[test]
fn update_falls_back_to_the_exit_code_when_git_said_nothing() {
    let sandbox = Sandbox::new("update-silent");
    let root = sandbox.dir("Suite");
    sandbox.dir("Suite/fennec-core/.git");
    let environ = vars!["FENNEC_SUITE" => root.display().to_string()];
    let (_calls, runner) = recorder(|_| (128, String::new(), String::new()));
    let outcome = launcher::update_suite(&conf!(), &environ, &sandbox.0, &runner);
    assert_eq!(outcome.code, 1);
    assert_eq!(stdout_lines(&outcome.lines), vec!["fail fennec-core: exit 128".to_string()]);
}

#[test]
fn update_says_so_when_the_suite_is_not_installed() {
    let sandbox = Sandbox::new("update-empty");
    let root = sandbox.dir("Suite");
    let environ = vars!["FENNEC_SUITE" => root.display().to_string()];
    let (calls, runner) = recorder(|_| (0, String::new(), String::new()));
    let outcome = launcher::update_suite(&conf!(), &environ, &sandbox.0, &runner);
    assert_eq!(outcome.code, 2);
    assert!(calls.borrow().is_empty(), "nothing should be pulled");
    assert!(stderr_lines(&outcome.lines)[0].contains("no Fennec suite checkouts found"));
}

#[test]
fn update_reaches_the_environment_setup_next_to_the_suite() {
    let sandbox = Sandbox::new("update-setup");
    let root = sandbox.dir("Suite");
    sandbox.dir("Suite/fennec-environment-setup/.git");
    sandbox.dir("Fennec-Environment-Setup/.git");
    let environ = vars!["FENNEC_SUITE" => root.display().to_string()];
    let (calls, runner) = recorder(|_| (0, "Already up to date.\n".to_string(), String::new()));
    let outcome = launcher::update_suite(&conf!(), &environ, &sandbox.0, &runner);
    assert_eq!(outcome.code, 0);
    let pulled = calls.borrow().clone();
    assert_eq!(pulled.len(), 2, "{pulled:?}");
    let second = pulled[1].clone();
    assert!(second.iter().any(|part| part.ends_with("Fennec-Environment-Setup")), "{second:?}");
    assert!(stdout_lines(&outcome.lines).contains(&"Updated 2 checkout(s).".to_string()),
            "{:?}", stdout_lines(&outcome.lines));
}

#[test]
fn update_pulls_the_engine_checkout_and_asks_for_a_rebuild_when_it_moved() {
    let sandbox = Sandbox::new("update-engine-moved");
    let root = sandbox.dir("Suite");
    sandbox.dir("Suite/fennec-core/.git");
    sandbox.dir("Fennec/.git");
    let environ = vars!["FENNEC_SUITE" => root.display().to_string()];
    let engine = sandbox.path("Fennec");
    let (calls, runner) = recorder(move |argv: &[String]| {
        if argv.iter().any(|part| part == &engine.display().to_string()) {
            (0, "Updating abc123..def456\nFast-forward\n 3 files changed, 4 insertions(+)\n".to_string(),
             String::new())
        } else {
            (0, "Already up to date.\n".to_string(), String::new())
        }
    });
    let outcome = launcher::update_suite(&conf!(), &environ, &sandbox.0, &runner);
    assert_eq!(outcome.code, 0);
    let pulled = calls.borrow().clone();
    assert_eq!(pulled.len(), 2, "the engine beside the suite is updated too: {pulled:?}");
    assert!(pulled.iter().any(|argv| argv.iter().any(|part| part.ends_with("Fennec"))),
            "{pulled:?}");
    let report = stdout_lines(&outcome.lines);
    assert!(report.iter().any(|line| line.starts_with("ok   Fennec: 3 files changed")),
            "{report:?}");
    assert!(report.iter().any(|line| line.contains("engine: source updated, binary not rebuilt")
        && line.contains("cargo build --release")), "{report:?}");
    assert!(report.contains(&"Updated 2 checkout(s).".to_string()), "{report:?}");
    assert!(stderr_lines(&outcome.lines).is_empty(), "{:?}", stderr_lines(&outcome.lines));
}

#[test]
fn a_still_up_to_date_engine_gets_no_rebuild_nag() {
    let sandbox = Sandbox::new("update-engine-current");
    let root = sandbox.dir("Suite");
    sandbox.dir("Fennec/.git");
    let environ = vars!["FENNEC_SUITE" => root.display().to_string()];
    let (_calls, runner) = recorder(|_| (0, "Already up to date.\n".to_string(), String::new()));
    let outcome = launcher::update_suite(&conf!(), &environ, &sandbox.0, &runner);
    assert_eq!(outcome.code, 0);
    let report = stdout_lines(&outcome.lines);
    assert!(report.iter().any(|line| line == "ok   Fennec: Already up to date."), "{report:?}");
    assert!(!report.iter().any(|line| line.contains("cargo build")), "{report:?}");
}

#[test]
fn the_engine_is_resolved_the_way_the_launcher_resolves_it() {
    let sandbox = Sandbox::new("update-engine-configured");
    let root = sandbox.dir("Suite");
    let engine = sandbox.dir("Elsewhere/Engine");
    sandbox.dir("Elsewhere/Engine/.git");
    let environ = vars!["FENNEC_SUITE" => root.display().to_string(),
                        "FENNEC_HOME" => engine.display().to_string()];
    let (calls, runner) = recorder(|_| (0, "Already up to date.\n".to_string(), String::new()));
    launcher::update_suite(&conf!(), &environ, &sandbox.0, &runner);
    let pulled = calls.borrow().clone();
    assert_eq!(pulled.len(), 1, "only the configured engine is pulled: {pulled:?}");
    assert!(pulled[0].iter().any(|part| part.ends_with("Engine")), "{pulled:?}");
}

#[test]
fn the_suite_list_is_the_repos_the_installer_clones() {
    // fennec-ui and fennec-economy are here because the crates above path-depend on them:
    // `fennec update` must not fast-forward a consumer while its dependency stays behind.
    assert_eq!(launcher::SUITE_REPOS.len(), 10);
    for name in ["fennec-desktop", "fennec-core", "fennec-cli", "fennec-installer",
                 "fennec-finetune", "fennec-jetbrains", "fennec-coding-agent",
                 "fennec-environment-setup", "fennec-ui", "fennec-economy"] {
        assert!(launcher::SUITE_REPOS.contains(&name), "{name} is not in the suite list");
    }
}

#[test]
fn a_pull_with_the_real_git_reports_instead_of_panicking() {
    // The injected runners cover the report shape; this one covers the real
    // spawn: no console window, no prompt, and a directory that is not a
    // checkout comes back as git's own failure rather than a hang.
    let sandbox = Sandbox::new("pull-real");
    let argv: Vec<String> = ["git", "-C", sandbox.0.to_str().unwrap(), "rev-parse", "--git-dir"]
        .iter().map(|part| part.to_string()).collect();
    let (code, _stdout, stderr) = launcher::pull(&argv);
    if code == 127 {
        return; // no git on this host
    }
    assert!(code == 0 || stderr.contains("not a git repository"), "exit {code}: {stderr}");
}

// --------------------------------------------------------------------- doctor

fn check<'a>(checks: &'a [Check], name: &str) -> &'a Check {
    checks.iter().find(|check| check.name == name)
        .unwrap_or_else(|| panic!("no {name} check in {checks:?}"))
}

#[test]
fn a_native_engine_report_says_python_is_not_needed() {
    let sandbox = Sandbox::new("doctor-native");
    let engine_home = sandbox.engine();
    let model = sandbox.dir("models/qwen");
    sandbox.dir("Suite/fennec-core/.git");
    sandbox.file("TE/tokeneconomizer/plugin/hook.py", "");
    let environ = vars!["FENNEC_HOME" => engine_home.display().to_string(),
                        "FENNEC_SUITE" => sandbox.path("Suite").display().to_string(),
                        "FENNEC_MODEL" => model.display().to_string(),
                        "FENNEC_TOKENECONOMIZER" => sandbox.path("TE").display().to_string()];
    let checks = launcher::doctor(&conf!(), &environ, &sandbox.0);
    assert!(!check(&checks, "config").ok);
    assert!(check(&checks, "config").detail.starts_with("not found:"));
    assert!(check(&checks, "python").ok);
    assert_eq!(check(&checks, "python").detail, "not needed: native Fennec binary");
    assert!(check(&checks, "engine").detail.starts_with("fennec "));
    assert!(check(&checks, "engine").detail.contains(binary_name()));
    assert!(check(&checks, "model").ok);
    assert!(check(&checks, "suite").ok);
    assert!(check(&checks, "suite").detail.contains("1 checkout(s)"));
    assert!(check(&checks, "tokeneconomizer").ok);
    assert_eq!(launcher::print_doctor(&checks), 0);
}

#[test]
fn a_missing_engine_fails_both_engine_and_python_and_exits_one() {
    let sandbox = Sandbox::new("doctor-none");
    let empty = sandbox.dir("Empty");
    let environ = vars!["FENNEC_HOME" => empty.display().to_string()];
    let checks = launcher::doctor(&conf!(), &environ, &sandbox.0);
    assert!(!check(&checks, "engine").ok);
    assert!(!check(&checks, "python").ok);
    assert_eq!(check(&checks, "engine").detail, check(&checks, "python").detail);
    assert_eq!(launcher::print_doctor(&checks), 1);
}

#[test]
fn a_model_that_is_not_on_disk_fails_only_its_own_check() {
    let sandbox = Sandbox::new("doctor-model");
    let engine_home = sandbox.engine();
    let environ = vars!["FENNEC_HOME" => engine_home.display().to_string(),
                        "FENNEC_MODEL" => sandbox.path("gone").display().to_string()];
    let checks = launcher::doctor(&conf!(), &environ, &sandbox.0);
    assert!(!check(&checks, "model").ok);
    assert!(check(&checks, "engine").ok);
    assert!(check(&checks, "suite").detail.contains("0 checkout(s)"));
    // A missing model is not a broken launcher: the report still exits 0.
    assert_eq!(launcher::print_doctor(&checks), 0);
    assert_eq!(check(&checks, "tokeneconomizer").detail, "not configured");
}

#[test]
fn the_hook_source_pointer_file_is_read_and_trimmed() {
    let sandbox = Sandbox::new("doctor-pointer");
    let engine_home = sandbox.engine();
    sandbox.file(".fennec/tokeneconomizer.path", "  C:/src/TokenEconomizer  \n");
    let environ = vars!["FENNEC_HOME" => engine_home.display().to_string()];
    let checks = launcher::doctor(&conf!(), &environ, &sandbox.0);
    assert!(!check(&checks, "tokeneconomizer").ok, "the pointer names a tree that is not here");
    assert_eq!(check(&checks, "tokeneconomizer").detail, "C:/src/TokenEconomizer");
    sandbox.file("TE/tokeneconomizer/plugin/hook.py", "");
    sandbox.file(".fennec/tokeneconomizer.path", &format!("  {}  \n", sandbox.path("TE").display()));
    let checks = launcher::doctor(&conf!(), &environ, &sandbox.0);
    assert!(check(&checks, "tokeneconomizer").ok);
}

#[test]
fn an_exported_hook_source_beats_the_pointer_file() {
    let sandbox = Sandbox::new("doctor-te-env");
    let engine_home = sandbox.engine();
    sandbox.file(".fennec/tokeneconomizer.path", "C:/elsewhere");
    sandbox.file("TE/tokeneconomizer/plugin/hook.py", "");
    let te = sandbox.path("TE").display().to_string();
    let environ = vars!["FENNEC_HOME" => engine_home.display().to_string(),
                        "FENNEC_TOKENECONOMIZER" => te.clone()];
    assert!(check(&launcher::doctor(&conf!(), &environ, &sandbox.0), "tokeneconomizer").ok);
    let environ = vars!["FENNEC_HOME" => engine_home.display().to_string(), "TE_CORE_PATH" => te];
    assert!(check(&launcher::doctor(&conf!(), &environ, &sandbox.0), "tokeneconomizer").ok);
}

// ---------------------------------------------------------------------- words

#[test]
fn the_help_lists_every_verb_and_the_engine_remedy() {
    for phrase in ["fennec update", "fennec diagnose", "fennec preflight", "fennec acp",
                   "fennec outlets", "fennec setup", "cargo build --release", "target/release",
                   "FENNEC_HOME"] {
        assert!(launcher::HELP.contains(phrase), "help does not mention {phrase}");
    }
    assert!(launcher::HELP.contains("no inference binaries or model weights"));
    assert!(launcher::NO_ENGINE.contains("Fennec engine not found"));
    assert!(launcher::NO_ENGINE.contains("installer"));
    assert!(launcher::NEEDS_CORE.contains("fennec-core"));
}

#[test]
fn the_python_snippets_survive_the_rust_line_continuations() {
    // Rust's `\` continuation drops the next line's leading whitespace, so an
    // indented block has to share its line with its header. These are the bytes
    // handed to `python -c`, pinned because a typo here is a SyntaxError in
    // someone else's process.
    assert_eq!(
        launcher::ACP_CHECK,
        "import importlib, json, sys\n\
         value = importlib.import_module(sys.argv[1]).check()\n\
         text = json.dumps(value, indent=2, ensure_ascii=False)\n\
         try: print(text)\n\
         except UnicodeEncodeError: print(json.dumps(value, indent=2))\n"
    );
    assert_eq!(
        launcher::OUTLETS_CHECK,
        "import importlib, json\n\
         loaded = importlib.import_module('config').load_config()\n\
         value = importlib.import_module('outlets').Outlets(loaded).status()\n\
         text = json.dumps(value, indent=2, ensure_ascii=False)\n\
         try: print(text)\n\
         except UnicodeEncodeError: print(json.dumps(value, indent=2))\n"
    );
    for snippet in [launcher::RESOLVE, launcher::WIZARD, launcher::ACP_PIPE, launcher::PROBE,
                    launcher::ACP_CHECK, launcher::OUTLETS_CHECK] {
        for line in snippet.lines() {
            assert!(!line.ends_with(':'), "an indented block would be flattened: {line}");
        }
        assert!(!snippet.contains("\n "), "{snippet}");
    }
    // The two that decide which `config.py` the handover gets, pinned byte for
    // byte: the prelude is what keeps the working directory out of the way, and
    // the probe is what keeps a stray namesake out of the interpreter.
    assert_eq!(
        launcher::RESOLVE,
        "import os as _os, sys as _sys; _core = [p for p in _os.environ.get('PYTHONPATH', '')\
         .split(_os.pathsep) if p]; _sys.path[:] = _core + [p for p in _sys.path \
         if p != '' and p not in _core]\n"
    );
    assert_eq!(
        launcher::PROBE,
        "import importlib.util as util, os, sys; spec = util.find_spec(sys.argv[1]); \
         origin = getattr(spec, 'origin', '') or ''; home = os.path.dirname(origin) if \
         origin.endswith('.py') else ''; core = bool(home) and all(os.path.isfile(os.path.join(home, \
         n + '.py')) for n in sys.argv[1:]); raise SystemExit(0 if core else 1)"
    );
}

// ----------------------------------------------------------------------- core

#[test]
fn core_candidates_are_exported_then_suite_then_this_binary_then_the_cwd() {
    let sandbox = Sandbox::new("core-order");
    let release = sandbox.path("Suite/fennec-cli/target/release");
    let environ = vars!["FENNEC_SUITE" => sandbox.path("Suite").display().to_string(),
                        "FENNEC_CORE" => sandbox.path("Elsewhere").display().to_string()];
    let candidates = launcher::core_candidates(&conf!(), &environ, &sandbox.0, &release);
    assert_eq!(candidates[0], sandbox.path("Elsewhere"));
    assert_eq!(candidates[1], sandbox.path("Suite/fennec-core"));
    assert_eq!(candidates[2], release.join("fennec-core"));
    assert_eq!(candidates[3], sandbox.path("Suite/fennec-cli/target/fennec-core"));
    assert_eq!(candidates[4], sandbox.path("Suite/fennec-cli/fennec-core"));
    assert_eq!(candidates.iter().filter(|path| *path == &sandbox.path("Suite/fennec-core")).count(), 1,
               "a repeated candidate is not probed twice");
    assert_eq!(candidates.last().unwrap(), &std::env::current_dir().unwrap().join("fennec-core"));
}

#[test]
fn a_blank_exported_core_path_is_not_a_candidate() {
    let sandbox = Sandbox::new("core-blank");
    let candidates = launcher::core_candidates(&conf!(), &vars!["FENNEC_CORE" => "  "], &sandbox.0, &sandbox.0);
    assert_eq!(candidates[0], sandbox.path("Fennec-Suite/fennec-core"));
    assert!(!candidates.iter().any(|path| path == &sandbox.path("Elsewhere")));
}

#[test]
fn a_core_target_uses_a_checkout_that_carries_every_module_it_needs() {
    let sandbox = Sandbox::new("core-checkout");
    let python = sandbox.file("bin/python", "");
    let root = sandbox.dir("Suite");
    sandbox.file("Suite/fennec-core/config.py", "");
    sandbox.file("Suite/fennec-core/outlets.py", "");
    let environ = vars!["FENNEC_SUITE" => root.display().to_string(),
                        "FENNEC_PYTHON" => python.display().to_string()];
    let target = launcher::core_target(&["config", "outlets"], &conf!(), &environ, &sandbox.0,
                                       &sandbox.0, "outlets").unwrap();
    assert_eq!(target.python, real(&python));
    assert_eq!(target.root, Some(root.join("fennec-core")));
    assert_eq!(target.module, "config");
}

#[test]
fn a_half_populated_checkout_is_not_a_core_target() {
    let sandbox = Sandbox::new("core-partial");
    let python = sandbox.file("bin/python", "");
    let root = sandbox.dir("Suite");
    sandbox.file("Suite/fennec-core/config.py", "");
    // The interpreter is a sandbox placeholder, so the import probe cannot
    // succeed: the answer has to be the "no core anywhere" message, not a
    // silently half-working handover.
    let environ = vars!["FENNEC_SUITE" => root.display().to_string(),
                        "FENNEC_PYTHON" => python.display().to_string()];
    let error = launcher::core_target(&["config", "outlets"], &conf!(), &environ, &sandbox.0,
                                      &sandbox.0, "outlets").unwrap_err();
    assert!(error.starts_with("fennec outlets "), "{error}");
    assert!(error.contains(launcher::NEEDS_CORE), "{error}");
    assert!(error.contains("looked in"), "{error}");
    assert!(error.contains("fennec-core"), "{error}");
}

#[test]
fn an_unusable_interpreter_is_reported_before_the_core_is_looked_for() {
    let sandbox = Sandbox::new("core-no-python");
    let missing = sandbox.path("bin/python");
    let error = launcher::core_target(&["acp"], &conf!(),
                                      &vars!["FENNEC_PYTHON" => missing.display().to_string()],
                                      &sandbox.0, &sandbox.0, "acp").unwrap_err();
    assert!(error.starts_with("configured Python interpreter does not exist:"), "{error}");
}

#[test]
fn a_core_handover_that_cannot_start_its_interpreter_exits_two() {
    // run_core's contract: stdio inherited, exit code propagated, and a process
    // that cannot start reported instead of swallowed. The placeholder cannot
    // start, so the failure path is the one under test.
    let sandbox = Sandbox::new("core-run");
    let target = launcher::CoreTarget {
        python: sandbox.file("bin/python", ""),
        root: Some(sandbox.path("Suite/fennec-core")),
        module: "acp".to_string(),
    };
    assert_eq!(launcher::run_core(&target, launcher::ACP_PIPE, &[]), 2);
}
