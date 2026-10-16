//! `sccache-r2`: see the README.

use sccache_r2::{Config, Outcome, render, render_shell};
use std::io::Write;
use std::process::{Command, ExitCode};

const USAGE: &str = "\
usage: sccache-r2 <command> [--read-only]

commands:
  env      print the sccache env as NAME=value lines (includes the credentials: redirect it)
  shell    print it as `export` lines for eval (includes the credentials)
  github   append it to $GITHUB_ENV, mask the credentials, write enabled=true|false to
           $GITHUB_OUTPUT; a missing backend clears RUSTC_WRAPPER and still succeeds
  check    validate the config, start sccache against the bucket and print its stats
  stats    print `sccache --show-stats`

--read-only  read the cache, never write it, e.g. on pull requests
             (also SCCACHE_R2_READ_ONLY=true)
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let read_only = args.iter().any(|a| a == "--read-only");
    let Some(cmd) = args.iter().find(|a| !a.starts_with("--")) else {
        eprint!("{USAGE}");
        return ExitCode::from(2);
    };
    match run(cmd, read_only) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("sccache-r2: {e}");
            ExitCode::from(1)
        }
    }
}

fn append(path_var: &str, text: &str) -> Result<(), String> {
    let path = std::env::var(path_var).map_err(|_| format!("{path_var} is not set"))?;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut f| f.write_all(text.as_bytes()))
        .map_err(|e| format!("{path}: {e}"))
}

fn report(outcome: &Outcome) {
    match outcome {
        Outcome::Enabled(c) => eprintln!(
            "sccache-r2: on, bucket {} ({})",
            c.bucket,
            if c.read_only {
                "read-only"
            } else {
                "read-write"
            }
        ),
        Outcome::Disabled { missing } => {
            eprintln!("sccache-r2: off, missing {}", missing.join(", "))
        }
    }
}

fn sccache(args: &[&str], env: &[(&str, String)]) -> Result<bool, String> {
    Command::new("sccache")
        .args(args)
        .envs(env.iter().map(|(k, v)| (*k, v.as_str())))
        .status()
        .map(|s| s.success())
        .map_err(|e| format!("cannot run sccache: {e}"))
}

fn code(ok: bool) -> ExitCode {
    if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn run(cmd: &str, read_only: bool) -> Result<ExitCode, String> {
    if cmd == "stats" {
        return Ok(code(sccache(&["--show-stats"], &[])?));
    }
    let outcome = Config::from_env(read_only).map_err(|e| e.to_string())?;
    let env = outcome.env();
    match cmd {
        "env" => print!("{}", render(&env)),
        "shell" => print!("{}", render_shell(&env)),
        "github" => {
            for (k, v) in &env {
                if Outcome::is_secret(k) && !v.is_empty() {
                    println!("::add-mask::{v}");
                }
            }
            append("GITHUB_ENV", &render(&env))?;
            append("GITHUB_OUTPUT", &format!("enabled={}\n", outcome.enabled()))?;
            if !outcome.enabled() {
                println!(
                    "::notice title=sccache::off: the R2 secrets or variables are not available"
                );
            }
        }
        "check" => {
            report(&outcome);
            if !outcome.enabled() {
                return Ok(ExitCode::from(1));
            }
            // A fresh server opens the bucket on start and refuses to start when it is
            // unreachable or the credentials are rejected.
            let _ = Command::new("sccache").arg("--stop-server").output();
            let ok = sccache(&["--start-server"], &env)? && sccache(&["--show-stats"], &env)?;
            return Ok(code(ok));
        }
        _ => {
            eprint!("{USAGE}");
            return Ok(ExitCode::from(2));
        }
    }
    report(&outcome);
    Ok(ExitCode::SUCCESS)
}
