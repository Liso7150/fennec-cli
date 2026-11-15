//! One crate, three names, one implementation of each behaviour.
//!
//! - `fennec` / `coli` - the launcher ([`launcher`]): it resolves the engine
//!   from the shared config and execs it, and answers the suite verbs
//!   (`update`, `diagnose`, `preflight`). `setup`, `acp` and `outlets` hand the
//!   process to `fennec_core`, which owns the first-run wizard, ACP v1 and the
//!   outlet probes. `coli` is the legacy engine's entry-point name.
//! - `fennec-rs` - the ported glue ([`glue`]) as a program, so it can be run,
//!   measured and served to Python over NDJSON without a CPython interpreter in
//!   the loop. The product verbs keep their Python contract: same flags, same
//!   bytes where bytes are the contract. The `bench` verbs replay a workload
//!   file and answer with the same JSON shapes the Python snippets in
//!   `benchmarks/bench.py` answer with, so `--impl python` and `--impl rust`
//!   time the same work the same way.
//!
//! The surface is fixed by the bin wrapper that started the process, so argv
//! cannot switch it: `fennec serve` reaches the engine while `fennec-rs serve`
//! stays the bridge `fennec_core/rust_bridge.py` talks to. Nothing here ships
//! inference binaries or model weights.

pub mod glue;
pub mod launcher;
pub mod mem;

use std::path::PathBuf;

/// Which half of the crate answers this process.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Surface {
    Launcher,
    Glue,
}

/// The directory the running executable sits in, which is where a
/// `fennec-core` checkout is expected to be found next to it.
fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(PathBuf::from))
        .unwrap_or_default()
}

/// The `fennec-rs` surface: the ported glue and the bench replay.
fn glue_code(argv: &[String]) -> i32 {
    let command = argv.first().map(String::as_str).unwrap_or("");
    let rest: &[String] = if argv.is_empty() { &[] } else { &argv[1..] };
    let no_config = glue::Map::new();
    match command {
        "--version" | "-V" | "version" => {
            println!("fennec-rs {}", glue::VERSION);
            0
        }
        "" | "--help" | "-h" | "help" => {
            print!("{}", glue::USAGE);
            if command.is_empty() { 2 } else { 0 }
        }
        "mem" => {
            println!("{}", glue::dumps(&mem::report()));
            0
        }
        "preflight" => glue::cmd_preflight(rest, &no_config),
        "compact" => glue::cmd_compact(rest),
        "elide" => glue::cmd_elide(rest),
        "redact" => glue::cmd_redact(),
        "urlgate" => glue::cmd_urlgate(rest),
        "frames" => glue::cmd_frames(),
        "acp" => glue::cmd_acp(),
        "serve" => glue::cmd_serve(),
        "bench" => glue::cmd_bench(rest),
        other => {
            eprintln!("fennec-rs: unknown command {other}\n\n{}", glue::USAGE);
            2
        }
    }
}

fn code(surface: Surface) -> i32 {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    match surface {
        Surface::Launcher => {
            launcher::dispatch(&argv, glue::VERSION, &exe_dir(), &glue::cmd_preflight)
        }
        Surface::Glue => glue_code(&argv),
    }
}

/// The three `src/bin` wrappers call this, so the exit code is the only thing
/// they differ in.
pub fn run(surface: Surface) -> ! {
    std::process::exit(code(surface))
}
