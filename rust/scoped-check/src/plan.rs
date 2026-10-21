//! Turns the change set into the commands to run. When the exact answer is out of reach the
//! plan records the error; it runs every gate only if the caller opted into the fallback.

use std::path::PathBuf;

use cargo_changed_packages::{Affected, Resolver};
use path_gates::Selection;
use serde::Serialize;

use crate::config::{Config, Gate, Part};
use crate::quote;

/// A command that will run.
#[derive(Serialize)]
pub struct Step {
    pub name: String,
    pub command: String,
    /// `scoped`, `all: unmatched <path>`, `all: fallback after error: <error>` or `--all`.
    pub why: String,
}

/// A selected gate that has nothing to do.
#[derive(Serialize)]
pub struct Skip {
    pub name: String,
    pub reason: String,
}

/// The outcome of planning; also the `plan --json` document.
#[derive(Serialize)]
pub struct Plan {
    pub base: String,
    pub merge_base: Option<String>,
    /// `None` when the change set could not be computed.
    pub changed: Option<usize>,
    pub nothing_changed: bool,
    pub gates: Vec<Step>,
    pub skipped: Vec<Skip>,
    /// Why planning failed; `null` when it did not. Without the opt-in fallback `gates` is
    /// then empty, with it `gates` holds every gate.
    pub error: Option<String>,
}

struct Values {
    packages: String,
    filter: String,
    changed: String,
}

/// Plan the gates for `base`. A planning error is recorded in [`Plan::error`] and never
/// turns into "nothing to do"; `fallback_all` makes it select every gate instead.
pub fn build(cfg: &Config, base: &str, all: bool, fallback_all: bool, resolver: &Resolver) -> Plan {
    let mut plan = Plan {
        base: base.to_owned(),
        merge_base: None,
        changed: None,
        nothing_changed: false,
        gates: Vec::new(),
        skipped: Vec::new(),
        error: None,
    };
    let mut forced = all.then(|| "--all".to_owned());
    let mut paths = Vec::new();
    match git_changed_paths::changed_paths(&cfg.dir, base) {
        Ok(found) => {
            plan.merge_base = Some(found.merge_base);
            paths = found.paths.into_iter().collect::<Vec<_>>();
            plan.changed = Some(paths.len());
        }
        // `--all` ignores the change set on purpose, so its failure is not an error then.
        Err(e) if forced.is_none() => {
            let changed = Err(UNKNOWN.to_owned());
            return failed(cfg, plan, &e.to_string(), fallback_all, &changed);
        }
        Err(_) => {}
    }
    // What `{changed}` becomes when every gate runs: every changed path, or why there is none.
    let changed = if plan.changed.is_none() {
        Err(UNKNOWN.to_owned())
    } else if paths.is_empty() {
        Err("nothing changed".to_owned())
    } else {
        quote(&paths.iter().collect::<Vec<_>>())
    };
    if forced.is_none() {
        if paths.is_empty() {
            plan.nothing_changed = true;
            return plan;
        }
        let selection = cfg.rules.select(&paths);
        if selection.all {
            let first = selection.unmatched.first().map(|p| p.display().to_string());
            forced = Some(format!("all: unmatched {}", first.unwrap_or_default()));
        } else {
            match scoped(cfg, &paths, &selection, resolver) {
                Ok((gates, skipped)) => {
                    plan.gates = gates;
                    plan.skipped = skipped;
                    return plan;
                }
                Err(e) => return failed(cfg, plan, &e, fallback_all, &changed),
            }
        }
    }
    (plan.gates, plan.skipped) = every_gate(cfg, &forced.unwrap_or_default(), &changed);
    plan
}

const UNKNOWN: &str = "the change set is unknown";

/// Every gate with full-workspace values. A gate using `{changed}` is skipped when `changed`
/// holds no paths: an empty argument list makes tools like `gofmt -w` read stdin and hang.
fn every_gate(cfg: &Config, why: &str, changed: &Result<String, String>) -> Scoped {
    let (mut steps, mut skipped) = (Vec::new(), Vec::new());
    for gate in &cfg.gates {
        let changed = match changed {
            Ok(quoted) => quoted.clone(),
            Err(e) if gate.needs_changed() => {
                skipped.push(Skip {
                    name: gate.name.clone(),
                    reason: format!("{{changed}} has no paths: {e}"),
                });
                continue;
            }
            Err(_) => String::new(),
        };
        let full = Values {
            packages: "--workspace".to_owned(),
            filter: String::new(),
            changed,
        };
        steps.push(step(gate, &full, why));
    }
    (steps, skipped)
}

fn failed(
    cfg: &Config,
    mut plan: Plan,
    error: &str,
    fallback_all: bool,
    changed: &Result<String, String>,
) -> Plan {
    if fallback_all {
        let why = format!("all: fallback after error: {error}");
        (plan.gates, plan.skipped) = every_gate(cfg, &why, changed);
    }
    plan.error = Some(error.to_owned());
    plan
}

fn step(gate: &Gate, values: &Values, why: &str) -> Step {
    let command = gate
        .run
        .iter()
        .map(|part| match part {
            Part::Text(t) => t,
            Part::Packages => &values.packages,
            Part::NextestFilter => &values.filter,
            Part::Changed => &values.changed,
        })
        .map(String::as_str)
        .collect();
    Step {
        name: gate.name.clone(),
        command,
        why: why.to_owned(),
    }
}

type Scoped = (Vec<Step>, Vec<Skip>);

fn scoped(
    cfg: &Config,
    paths: &[PathBuf],
    selection: &Selection,
    resolver: &Resolver,
) -> Result<Scoped, String> {
    let (mut steps, mut skipped) = (Vec::new(), Vec::new());
    for gate in cfg
        .gates
        .iter()
        .filter(|g| selection.gates.contains(&g.name))
    {
        // Selecting each path alone is how a gate learns which paths it claims.
        let claimed: Vec<&PathBuf> = paths
            .iter()
            .filter(|p| cfg.rules.select([p]).gates.contains(&gate.name))
            .collect();
        let mut values = Values {
            packages: String::new(),
            filter: String::new(),
            changed: quote(&claimed)?,
        };
        if gate.needs_packages() {
            let affected = affected(cfg, resolver, &claimed)?;
            if !affected.all && affected.packages.is_empty() {
                skipped.push(Skip {
                    name: gate.name.clone(),
                    reason: "nothing it covers changed".to_owned(),
                });
                continue;
            }
            match affected.nextest_filter() {
                None => values.packages = "--workspace".to_owned(),
                Some(filter) => {
                    let flags = affected.packages.iter().flat_map(|p| ["-p", p.as_str()]);
                    values.packages = quote::join(flags)?;
                    values.filter = format!("-E {}", quote::quote(&filter)?);
                }
            }
        }
        steps.push(step(gate, &values, "scoped"));
    }
    Ok((steps, skipped))
}

/// The paths shell-quoted and joined with spaces.
fn quote(paths: &[&PathBuf]) -> Result<String, String> {
    let words: Option<Vec<&str>> = paths.iter().map(|p| p.to_str()).collect();
    let words = words.ok_or("a changed path is not valid UTF-8")?;
    quote::join(words).map_err(|e| format!("cannot quote a changed path: {e}"))
}

fn affected(cfg: &Config, resolver: &Resolver, claimed: &[&PathBuf]) -> Result<Affected, String> {
    // Changed paths are repo-relative; the cargo library wants them relative to the
    // workspace, so those outside it cannot affect its packages.
    let relative: Vec<PathBuf> = claimed
        .iter()
        .filter_map(|p| p.strip_prefix(&cfg.workspace).ok())
        .map(PathBuf::from)
        .collect();
    // One resolver serves every gate, so `cargo metadata` runs at most once per run.
    resolver.affected(&relative).map_err(|e| e.to_string())
}
