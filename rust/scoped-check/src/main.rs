//! `scoped-check`: run only the check commands a change touches.
//!
//! Exit codes: 0 success; the first failing gate's code (1 when it has none) when a gate
//! fails; 2 for config and usage errors; 3 when planning fails (git, base ref, merge base,
//! `cargo metadata`).

mod config;
mod plan;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use anyhow::Result;
use cargo_changed_packages::{CacheStatus, Resolver};
use clap::{Args, Parser, Subcommand};

use config::{Config, DEFAULT_BASE};
use plan::Plan;

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Args)]
struct Common {
    /// Ref to compare against [default: `base` in the config, else origin/main]
    #[arg(long)]
    base: Option<String>,
    /// Config file [default: scoped-check.toml at the git top level]
    #[arg(long)]
    config: Option<PathBuf>,
    /// Select every gate with full-workspace values, ignoring the change set
    #[arg(long)]
    all: bool,
    /// On a planning error run every gate (loud warning, exit by the gates) instead of
    /// failing with exit 3 [config: `fallback = "all"`]
    #[arg(long)]
    fallback_all: bool,
    /// Do not read or write the cargo metadata cache
    #[arg(long)]
    no_cache: bool,
    /// Report on stderr whether `cargo metadata` ran or the cache answered
    #[arg(long, short)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print the gates that would run and their expanded commands
    Plan {
        #[command(flatten)]
        common: Common,
        /// Print one JSON object instead of text
        #[arg(long)]
        json: bool,
    },
    /// Run the selected gates in config order
    Run {
        #[command(flatten)]
        common: Common,
        /// Keep running after a gate fails
        #[arg(long)]
        keep_going: bool,
    },
}

/// Planning failed (git, base ref, merge base, `cargo metadata`). Distinct from 2 (config and
/// usage) and from any gate's own exit code.
const EXIT_PLANNING: u8 = 3;

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("scoped-check: {e:#}");
            ExitCode::from(2)
        }
    }
}

// Strict by default: CI that cannot compute the change set must go red, because a warning
// plus exit 0 hides a broken pipeline for as long as nobody reads the log. Running every
// gate is still sound (it never skips work), so it stays available as an explicit opt-in
// (`--fallback-all` or `fallback = "all"`), for repositories that prefer a slow green build
// to a red one; it then warns loudly and writes the GitHub step summary.
fn run(cli: Cli) -> Result<ExitCode> {
    let (common, json, keep_going, execute) = match cli.command {
        Cmd::Plan { common, json } => (common, json, false, false),
        Cmd::Run { common, keep_going } => (common, false, keep_going, true),
    };
    let cfg = Config::load(&common.config.unwrap_or_else(default_config))?;
    let base = common
        .base
        .or_else(|| cfg.base.clone())
        .unwrap_or_else(|| DEFAULT_BASE.to_owned());
    let fallback_all = common.fallback_all || cfg.fallback_all;
    let mut resolver = Resolver::new(&cfg.dir.join(&cfg.workspace));
    if let Some(cargo) = std::env::var_os("CARGO") {
        resolver = resolver.cargo(cargo);
    }
    if let Some(dir) = cache_dir().filter(|_| !common.no_cache) {
        resolver = resolver.cache_dir(dir);
    }
    let plan = plan::build(&cfg, &base, common.all, fallback_all, &resolver);
    report_metadata(resolver.cache_status(), common.verbose);
    if let Some(error) = &plan.error {
        if !fallback_all {
            if json {
                println!("{}", serde_json::to_string_pretty(&plan)?);
            }
            eprintln!("scoped-check: error: {error}");
            eprintln!(
                "scoped-check: no gate was run; fix the cause, or opt into running every gate \
                 with --fallback-all"
            );
            return Ok(ExitCode::from(EXIT_PLANNING));
        }
        loud_fallback(error);
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&plan)?);
        return Ok(ExitCode::SUCCESS);
    }
    if plan.nothing_changed {
        println!("nothing changed against {base}");
        return Ok(ExitCode::SUCCESS);
    }
    Ok(if execute {
        execute_plan(&cfg, &plan, keep_going)
    } else {
        print_plan(&plan);
        ExitCode::SUCCESS
    })
}

/// `SCOPED_CHECK_CACHE_DIR`, else `$XDG_CACHE_HOME/scoped-check`, else
/// `~/.cache/scoped-check`; `None` (no caching) when none of them is known.
fn cache_dir() -> Option<PathBuf> {
    let var = |name| std::env::var_os(name).filter(|v| !v.is_empty());
    var("SCOPED_CHECK_CACHE_DIR")
        .map(PathBuf::from)
        .or_else(|| var("XDG_CACHE_HOME").map(|d| PathBuf::from(d).join("scoped-check")))
        .or_else(|| var("HOME").map(|d| PathBuf::from(d).join(".cache/scoped-check")))
}

/// A failed cache write is always shown (it makes every run slow); the rest only with
/// `--verbose`.
fn report_metadata(status: Option<&CacheStatus>, verbose: bool) {
    let line = match status {
        None => "cargo metadata: not needed",
        Some(CacheStatus::Hit) => "cargo metadata: skipped (cache hit)",
        Some(CacheStatus::Miss) => "cargo metadata: ran (cache miss, stored)",
        Some(CacheStatus::Disabled) => "cargo metadata: ran (cache disabled)",
        Some(CacheStatus::MissNotStored(why)) => {
            eprintln!("scoped-check: warning: metadata cache not stored: {why}");
            return;
        }
    };
    if verbose {
        eprintln!("scoped-check: {line}");
    }
}

/// The opt-in fallback must not pass unnoticed: stderr, and the GitHub step summary when set.
fn loud_fallback(error: &str) {
    let text = format!(
        "scoped-check: WARNING: planning failed ({error}); running EVERY gate because the \
         fallback is enabled"
    );
    eprintln!("{text}");
    if let Some(file) = std::env::var_os("GITHUB_STEP_SUMMARY") {
        append_line(Path::new(&file), &format!("**{text}**"));
    }
}

fn append_line(file: &Path, line: &str) {
    let result = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
        .and_then(|mut f| writeln!(f, "{line}\n"));
    if let Err(e) = result {
        eprintln!("scoped-check: cannot write {}: {e}", file.display());
    }
}

fn default_config() -> PathBuf {
    // Outside git the current directory is the best guess; a missing config is then a
    // config error (exit 2), and with a config, planning fails with exit 3.
    let top = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| PathBuf::from(s.trim_end_matches('\n')));
    top.unwrap_or_default().join("scoped-check.toml")
}

fn print_plan(plan: &Plan) {
    println!("base: {}", plan.base);
    println!(
        "merge base: {}",
        plan.merge_base.as_deref().unwrap_or("unknown")
    );
    match plan.changed {
        Some(n) => println!("changed paths: {n}"),
        None => println!("changed paths: unknown"),
    }
    for step in &plan.gates {
        println!("gate {} [{}]: {}", step.name, step.why, step.command);
    }
    for skip in &plan.skipped {
        println!("skip {}: {}", skip.name, skip.reason);
    }
}

fn execute_plan(cfg: &Config, plan: &Plan, keep_going: bool) -> ExitCode {
    for skip in &plan.skipped {
        println!("== {}: skipped, {}", skip.name, skip.reason);
    }
    let mut first_failure = None;
    let mut failed = Vec::new();
    for step in &plan.gates {
        println!("== {}: {}", step.name, step.command);
        let code = shell(&step.command, cfg).unwrap_or_else(|e| {
            eprintln!("scoped-check: cannot start `{}`: {e}", step.name);
            1
        });
        if code != 0 {
            first_failure.get_or_insert(code);
            failed.push(step.name.as_str());
            if !keep_going {
                break;
            }
        }
    }
    if keep_going && !failed.is_empty() {
        eprintln!(
            "scoped-check: {} gate(s) failed: {}",
            failed.len(),
            failed.join(", ")
        );
    }
    // The first failing gate's own code (1 when it has none), also under --keep-going.
    ExitCode::from(first_failure.unwrap_or(0))
}

/// Exit code of `command` under the platform shell; 1 when it has none (killed by a signal).
fn shell(command: &str, cfg: &Config) -> std::io::Result<u8> {
    let (program, flag) = if cfg!(windows) {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    };
    let status = Command::new(program)
        .arg(flag)
        .arg(command)
        .current_dir(&cfg.dir)
        .status()?;
    Ok(status.code().map_or(1, |c| u8::try_from(c).unwrap_or(1)))
}
