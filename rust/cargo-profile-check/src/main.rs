use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use cargo_profile_check::{
    BUNDLED_TEMPLATE, Error, Report, check_path, check_workspace, parse_template, render_json,
    render_text,
};
use clap::{Parser, ValueEnum};

#[derive(Clone, Copy, ValueEnum)]
enum Format {
    Text,
    Json,
}

/// Check Cargo.toml [profile.*] sections against a profile template.
///
/// Exit status: 0 no differences, 1 differences found, 2 usage or parse error.
#[derive(Parser)]
#[command(name = "cargo-profile-check", bin_name = "cargo profile-check", version, color = clap::ColorChoice::Never)]
struct Cli {
    /// Template TOML with [profile.*] sections (default: the bundled template).
    #[arg(long)]
    template: Option<PathBuf>,
    /// Walk this directory and check every workspace root and standalone package.
    #[arg(long)]
    workspace: Option<PathBuf>,
    /// With --workspace: skip this directory (relative to the walk root). Repeatable.
    #[arg(long)]
    exclude: Vec<PathBuf>,
    /// Output format.
    #[arg(long, value_enum, default_value = "text")]
    format: Format,
    /// Print the bundled template and exit.
    #[arg(long)]
    print_template: bool,
    /// Cargo.toml files or crate directories to check.
    paths: Vec<PathBuf>,
}

fn run(cli: Cli) -> Result<Vec<Report>, Error> {
    let text = match &cli.template {
        Some(p) => fs::read_to_string(p).map_err(|e| Error(format!("{}: {e}", p.display())))?,
        None => BUNDLED_TEMPLATE.to_string(),
    };
    let template = parse_template(&text)?;
    let mut reports = Vec::new();
    if let Some(ws) = &cli.workspace {
        reports.extend(check_workspace(ws, &cli.exclude, &template)?);
    }
    for p in &cli.paths {
        reports.push(check_path(p, &template)?);
    }
    if cli.workspace.is_none() && cli.paths.is_empty() {
        reports.extend(check_workspace(
            &PathBuf::from("."),
            &cli.exclude,
            &template,
        )?);
    }
    Ok(reports)
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().collect();
    // `cargo profile-check ...` runs us as `cargo-profile-check profile-check ...`.
    if args.get(1).map(String::as_str) == Some("profile-check") {
        args.remove(1);
    }
    let cli = match Cli::try_parse_from(args) {
        Ok(c) => c,
        Err(e) => {
            let code = if e.use_stderr() { 2 } else { 0 };
            let _ = e.print();
            return ExitCode::from(code);
        }
    };
    if cli.print_template {
        print!("{BUNDLED_TEMPLATE}");
        return ExitCode::SUCCESS;
    }
    let format = cli.format;
    match run(cli) {
        Ok(reports) => {
            let out = match format {
                Format::Text => render_text(&reports),
                Format::Json => render_json(&reports),
            };
            print!("{out}");
            if reports.iter().any(|r| !r.diffs.is_empty()) {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(e) => {
            eprintln!("ERROR {e}");
            ExitCode::from(2)
        }
    }
}
