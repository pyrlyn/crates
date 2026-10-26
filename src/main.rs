//! target-prune: keep the last N cargo build runs (plus everything they depend on) in a
//! `target/` dir and drop the rest. Dry run by default; nothing is deleted without `--apply`.
use clap::{Args, Parser, Subcommand};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Per-profile subdirs whose entries carry a unit hash (`libfoo-<hash>.rlib`, `build/foo-<hash>/`).
const ARTIFACT_DIRS: [&str; 4] = ["deps", "build", "incremental", "examples"];
/// File in the target root where `run` records the start time of every build it wraps.
const MARKERS: &str = ".target-prune-runs";

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Prune profile dirs under ROOTS (default ~/Grok/worktrees).
    Prune(Opts),
    /// Run a build command, then prune its target dir: `target-prune run --apply -- cargo build`.
    Run {
        #[command(flatten)]
        opts: Opts,
        /// The command to run (after `--`).
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },
    /// Print one JSON line per profile dir: sizes per subdir, unit/duplicate/session counts.
    Snapshot { roots: Vec<PathBuf> },
}

#[derive(Args, Clone)]
struct Opts {
    /// Dirs to scan (target dirs or anything above them).
    roots: Vec<PathBuf>,
    /// Build runs to keep.
    #[arg(long, default_value_t = 2)]
    keep: usize,
    /// Seconds of quiet that separate two runs when no `run` markers exist.
    #[arg(long, default_value_t = 60.0)]
    gap: f64,
    /// Really delete (default: dry run).
    #[arg(long)]
    apply: bool,
    /// List every path.
    #[arg(short, long)]
    verbose: bool,
}

/// One `.fingerprint/<name>-<hash>` dir: a lib, bin, test, build script or its run.
#[derive(Debug, Default)]
struct Unit {
    hash: String,
    started: f64,
    fps: HashSet<u64>,
    deps: Vec<(String, u64)>,
    names: HashSet<String>,
    files: Vec<PathBuf>,
}

fn mtime(p: &Path) -> f64 {
    fs::symlink_metadata(p)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0.0, |d| d.as_secs_f64())
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64()
}

/// Bytes on disk (allocated blocks), not following symlinks.
fn size_of(p: &Path) -> u64 {
    let Ok(m) = fs::symlink_metadata(p) else { return 0 };
    let own = m.blocks() * 512;
    if !m.is_dir() {
        return own;
    }
    own + fs::read_dir(p).map_or(0, |rd| rd.flatten().map(|e| size_of(&e.path())).sum())
}

fn read_dir_sorted(p: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = fs::read_dir(p).map_or(vec![], |rd| rd.flatten().map(|e| e.path()).collect());
    v.sort();
    v
}

fn file_name(p: &Path) -> &str {
    p.file_name().and_then(|s| s.to_str()).unwrap_or("")
}

/// The hash in `libfoo-<hash>.rlib`, `foo-<hash>.d`, `build/foo-<hash>/`.
fn hash_of(name: &str) -> &str {
    let stem = name.split('.').next().unwrap_or(name);
    stem.rsplit('-').next().unwrap_or(stem)
}

/// Cargo writes a unit's u64 fingerprint as 16 hex chars; accept both byte orders.
fn hex_to_u64s(text: &str) -> Vec<u64> {
    let t = text.trim();
    if t.len() != 16 {
        return vec![];
    }
    let mut b = [0u8; 8];
    for (i, byte) in b.iter_mut().enumerate() {
        match u8::from_str_radix(&t[2 * i..2 * i + 2], 16) {
            Ok(x) => *byte = x,
            Err(_) => return vec![],
        }
    }
    vec![u64::from_le_bytes(b), u64::from_be_bytes(b)]
}

/// Every dir holding a `.fingerprint`: `target/debug`, `target/<triple>/release`, ...
fn find_profile_dirs(roots: &[PathBuf]) -> Vec<PathBuf> {
    fn walk(d: &Path, out: &mut Vec<PathBuf>) {
        if d.join(".fingerprint").is_dir() {
            out.push(d.to_path_buf());
            return;
        }
        for e in read_dir_sorted(d) {
            let n = file_name(&e);
            let is_dir = fs::symlink_metadata(&e).map(|m| m.is_dir()).unwrap_or(false);
            if is_dir && !n.starts_with('.') && n != "node_modules" {
                walk(&e, out);
            }
        }
    }
    let mut out = vec![];
    for r in roots {
        walk(r, &mut out);
    }
    out
}

fn load_units(profile: &Path) -> HashMap<String, Unit> {
    let mut units = HashMap::new();
    for d in read_dir_sorted(&profile.join(".fingerprint")) {
        let name = file_name(&d).to_string();
        if !d.is_dir() || !name.contains('-') {
            continue;
        }
        let h = name.rsplit('-').next().unwrap().to_string();
        let mut u = Unit { hash: h.clone(), started: mtime(&d), files: vec![d.clone()], ..Default::default() };
        for f in read_dir_sorted(&d) {
            let n = file_name(&f).to_string();
            u.started = u.started.max(mtime(&f));
            if n.ends_with(".json") {
                let Ok(text) = fs::read_to_string(&f) else { continue };
                let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
                for dep in v["deps"].as_array().into_iter().flatten() {
                    let a = dep.as_array();
                    let name = a.and_then(|a| a.get(1)).and_then(|x| x.as_str());
                    let fp = a.and_then(|a| a.last()).and_then(|x| x.as_u64());
                    if let (Some(name), Some(fp)) = (name, fp) {
                        u.deps.push((name.to_string(), fp));
                    }
                }
            } else if !["dep-", "invoked", "output-", "stderr-", "root-output"].iter().any(|p| n.starts_with(p)) {
                u.names.insert(if n.contains("build-script") {
                    "build_script_build".into()
                } else {
                    n.split_once('-').map_or(n.clone(), |x| x.1.to_string())
                });
                if let Ok(t) = fs::read_to_string(&f) {
                    u.fps.extend(hex_to_u64s(&t));
                }
            }
        }
        units.insert(h, u);
    }
    for sub in ARTIFACT_DIRS {
        for e in read_dir_sorted(&profile.join(sub)) {
            if let Some(u) = units.get_mut(hash_of(file_name(&e))) {
                u.files.push(e);
            }
        }
    }
    units
}

/// Start times written by `target-prune run`, looked up in the profile dir and two parents.
fn load_markers(profile: &Path) -> Vec<f64> {
    profile
        .ancestors()
        .take(3)
        .find_map(|d| fs::read_to_string(d.join(MARKERS)).ok())
        .map(|t| t.lines().filter_map(|l| l.trim().parse().ok()).collect())
        .unwrap_or_default()
}

/// Cluster units by start time. A new run starts after a quiet gap longer than `gap`
/// seconds, or when a `run` marker falls between two consecutive units.
fn group_runs(units: &HashMap<String, Unit>, gap: f64, markers: &[f64]) -> Vec<Vec<String>> {
    let mut markers = markers.to_vec();
    markers.sort_by(f64::total_cmp);
    let markers = &markers[..];
    let mut v: Vec<&Unit> = units.values().collect();
    v.sort_by(|a, b| a.started.total_cmp(&b.started));
    let mut runs: Vec<Vec<String>> = vec![];
    let mut last: Option<f64> = None;
    for u in v {
        let split = match last {
            None => true,
            // With `run` markers, only markers split (a single crate can compile for longer
            // than `gap`); the gap rule still applies to units older than the first marker.
            Some(l) => {
                let by_marker = markers.iter().any(|&m| m > l && m <= u.started);
                let gap_applies = markers.first().is_none_or(|&m0| u.started < m0);
                by_marker || (gap_applies && u.started - l > gap)
            }
        };
        if split {
            runs.push(vec![]);
        }
        runs.last_mut().unwrap().push(u.hash.clone());
        last = Some(u.started);
    }
    runs
}

/// Kept roots plus everything they transitively depend on; second value counts deps that
/// map to no unit at all.
fn closure(units: &HashMap<String, Unit>, roots: &[String]) -> (HashSet<String>, usize) {
    let mut by_fp: HashMap<u64, &str> = HashMap::new();
    let mut by_name: HashMap<&str, Vec<&str>> = HashMap::new();
    for u in units.values() {
        for &fp in &u.fps {
            by_fp.insert(fp, &u.hash);
        }
        for n in &u.names {
            by_name.entry(n).or_default().push(&u.hash);
        }
    }
    let (mut keep, mut stack, mut unresolved) = (HashSet::new(), roots.to_vec(), 0);
    while let Some(h) = stack.pop() {
        if !keep.insert(h.clone()) {
            continue;
        }
        for (name, fp) in &units[&h].deps {
            if let Some(d) = by_fp.get(fp) {
                stack.push(d.to_string());
            } else if let Some(hs) = by_name.get(name.as_str()) {
                // The dep was rebuilt since: keep every unit of that crate rather than guess.
                stack.extend(hs.iter().map(|s| s.to_string()));
            } else {
                unresolved += 1;
            }
        }
    }
    (keep, unresolved)
}

/// Sessions of one `incremental/<crate>-<id>` dir, oldest first.
fn sessions(inc: &Path) -> Vec<PathBuf> {
    let mut s: Vec<PathBuf> = read_dir_sorted(inc).into_iter().filter(|p| p.is_dir()).collect();
    s.sort_by(|a, b| mtime(a).total_cmp(&mtime(b)));
    s
}

fn with_lock(s: &Path, out: &mut Vec<PathBuf>) {
    out.push(s.to_path_buf());
    let lock = s.with_file_name(format!("{}.lock", file_name(s)));
    if lock.exists() {
        out.push(lock);
    }
}

struct Plan {
    units: usize,
    runs: usize,
    kept: usize,
    unresolved: usize,
    doomed: Vec<PathBuf>,
}

fn plan(profile: &Path, keep_runs: usize, gap: f64) -> Plan {
    let units = load_units(profile);
    let runs = group_runs(&units, gap, &load_markers(profile));
    let roots: Vec<String> = runs[runs.len().saturating_sub(keep_runs)..].concat();
    let (kept, unresolved) = closure(&units, &roots);
    let mut p = Plan { units: units.len(), runs: runs.len(), kept: kept.len(), unresolved, doomed: vec![] };
    if unresolved > 0 {
        p.kept = units.len();
        return p; // fail safe: a kept unit needs something we cannot map
    }
    let mut claimed: HashSet<PathBuf> = HashSet::new();
    for (h, u) in &units {
        claimed.extend(u.files.iter().cloned());
        if !kept.contains(h) {
            p.doomed.extend(u.files.iter().cloned());
        }
    }
    // incremental/<crate>-<id> is named by rustc's crate id, not cargo's unit hash, so it
    // rarely matches a unit. Keep only the newest session of dirs used since the oldest
    // kept run started; drop dirs idle since then.
    let cutoff = roots.iter().map(|h| units[h].started).fold(f64::INFINITY, f64::min) - gap;
    for inc in read_dir_sorted(&profile.join("incremental")) {
        if !inc.is_dir() || (claimed.contains(&inc) && !kept.contains(hash_of(file_name(&inc)))) {
            continue; // already doomed with its unit
        }
        let s = sessions(&inc);
        let newest = s.last().map_or(mtime(&inc), |l| mtime(l));
        if !claimed.contains(&inc) && newest < cutoff {
            p.doomed.push(inc);
        } else {
            for old in &s[..s.len().saturating_sub(1)] {
                with_lock(old, &mut p.doomed);
            }
        }
    }
    p
}

fn remove(p: &Path) {
    let is_dir = fs::symlink_metadata(p).map(|m| m.is_dir()).unwrap_or(false);
    let _ = if is_dir { fs::remove_dir_all(p) } else { fs::remove_file(p) };
}

fn human(n: u64) -> String {
    let mut x = n as f64;
    for u in ["B", "KB", "MB", "GB"] {
        if x < 1024.0 {
            return format!("{x:.1} {u}");
        }
        x /= 1024.0;
    }
    format!("{x:.1} TB")
}

fn prune(o: &Opts) -> (u64, u64) {
    let (mut total_all, mut free_all) = (0, 0);
    let verb = if o.apply { "freed" } else { "would free" };
    for profile in find_profile_dirs(&o.roots) {
        let p = plan(&profile, o.keep, o.gap);
        let total: u64 = [".fingerprint"].iter().chain(ARTIFACT_DIRS.iter()).map(|s| size_of(&profile.join(s))).sum();
        let free: u64 = p.doomed.iter().map(|d| size_of(d)).sum();
        total_all += total;
        free_all += free;
        if p.unresolved > 0 {
            eprintln!("{}: {} unresolved deps, skipping (fail safe)", profile.display(), p.unresolved);
        }
        println!(
            "{}: units {}, runs {}, kept units {}; total {}, {verb} {}, kept {}",
            profile.display(), p.units, p.runs, p.kept, human(total), human(free), human(total - free.min(total))
        );
        for d in &p.doomed {
            if o.verbose {
                println!("  - {}", d.display());
            }
            if o.apply {
                remove(d);
            }
        }
    }
    let tail = if o.apply { "" } else { "  (dry run; pass --apply to delete)" };
    println!("TOTAL: {}, {verb} {}{tail}", human(total_all), human(free_all));
    (total_all, free_all)
}

fn snapshot(roots: &[PathBuf]) {
    for profile in find_profile_dirs(roots) {
        let units = load_units(&profile);
        let mut per_crate: BTreeMap<String, usize> = BTreeMap::new();
        for d in read_dir_sorted(&profile.join(".fingerprint")) {
            let n = file_name(&d);
            *per_crate.entry(n.rsplit_once('-').map_or(n, |x| x.0).to_string()).or_default() += 1;
        }
        let mut dups: Vec<(&String, &usize)> = per_crate.iter().filter(|x| *x.1 > 1).collect();
        dups.sort_by(|a, b| b.1.cmp(a.1));
        let inc = read_dir_sorted(&profile.join("incremental"));
        let inc_dirs: Vec<&PathBuf> = inc.iter().filter(|p| p.is_dir()).collect();
        let sess: usize = inc_dirs.iter().map(|d| sessions(d).len()).sum();
        let mut sizes = serde_json::Map::new();
        for s in [".fingerprint"].iter().chain(ARTIFACT_DIRS.iter()) {
            sizes.insert(s.to_string(), (size_of(&profile.join(s)) >> 20).into());
        }
        let deps = read_dir_sorted(&profile.join("deps"));
        let count = |ext: &str| deps.iter().filter(|p| file_name(p).ends_with(ext)).count();
        let exe = deps.iter().filter(|p| !file_name(p).contains('.')).count();
        let line = serde_json::json!({
            "profile": profile.display().to_string(),
            "mb": sizes,
            "units": units.len(),
            "crates_with_dup_hashes": dups.len(),
            "dup_units": dups.iter().map(|x| x.1 - 1).sum::<usize>(),
            "top_dups": dups.iter().take(8).map(|(n, c)| format!("{n}x{c}")).collect::<Vec<_>>(),
            "rlib": count(".rlib"), "rmeta": count(".rmeta"), "d": count(".d"), "dylib": count(".dylib"),
            "exe": exe, "dsym": count(".dSYM"),
            "incr_dirs": inc_dirs.len(), "incr_sessions": sess,
        });
        println!("{line}");
    }
}

fn default_roots(r: &[PathBuf], fallback: PathBuf) -> Vec<PathBuf> {
    if r.is_empty() { vec![fallback] } else { r.to_vec() }
}

fn main() -> ExitCode {
    match Cli::parse().cmd {
        Cmd::Prune(mut o) => {
            let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default();
            o.roots = default_roots(&o.roots, home.join("Grok/worktrees"));
            prune(&o);
            ExitCode::SUCCESS
        }
        Cmd::Snapshot { roots } => {
            snapshot(&default_roots(&roots, PathBuf::from("target")));
            ExitCode::SUCCESS
        }
        Cmd::Run { mut opts, command } => {
            let target = std::env::var_os("CARGO_TARGET_DIR").map_or(PathBuf::from("target"), PathBuf::from);
            opts.roots = default_roots(&opts.roots, target);
            for r in &opts.roots {
                let _ = fs::create_dir_all(r);
                let line = format!("{:.3}\n", now());
                let path = r.join(MARKERS);
                let old = fs::read_to_string(&path).unwrap_or_default();
                let _ = fs::write(&path, old + &line);
            }
            let status = Command::new(&command[0]).args(&command[1..]).status();
            let code = match status {
                Ok(s) if s.success() => 0,
                Ok(s) => s.code().unwrap_or(1),
                Err(e) => {
                    eprintln!("target-prune: cannot run {}: {e}", command[0]);
                    127
                }
            };
            if code != 0 {
                eprintln!("target-prune: build failed (exit {code}); not pruning");
                return ExitCode::from(code as u8);
            }
            let t = Instant::now();
            prune(&opts);
            eprintln!("target-prune: prune took {:.2}s", t.elapsed().as_secs_f64());
            ExitCode::SUCCESS
        }
    }
}

#[cfg(test)]
mod tests;
