use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

static N: AtomicUsize = AtomicUsize::new(0);
const BASE: f64 = 1_000_000.0;

fn set_mtime(p: &Path, t: f64) {
    let f = fs::File::options().read(true).open(p).unwrap();
    f.set_modified(UNIX_EPOCH + std::time::Duration::from_secs_f64(t)).unwrap();
}

/// A fake unit the way cargo lays it out: fingerprint dir, rlib, .d.
fn unit(profile: &Path, name: &str, h: &str, fp: u64, deps: &[(&str, u64)], t: f64) {
    let d = profile.join(".fingerprint").join(format!("{name}-{h}"));
    fs::create_dir_all(&d).unwrap();
    fs::create_dir_all(profile.join("deps")).unwrap();
    let hex: String = fp.to_le_bytes().iter().map(|b| format!("{b:02x}")).collect();
    fs::write(d.join(format!("lib-{name}")), hex).unwrap();
    let deps: Vec<_> = deps.iter().map(|(n, f)| serde_json::json!([0, n, false, f])).collect();
    fs::write(d.join(format!("lib-{name}.json")), serde_json::json!({ "deps": deps }).to_string()).unwrap();
    fs::write(d.join("invoked.timestamp"), "x").unwrap();
    let rlib = profile.join("deps").join(format!("lib{name}-{h}.rlib"));
    fs::write(&rlib, vec![b'0'; 5000]).unwrap();
    fs::write(profile.join("deps").join(format!("{name}-{h}.d")), "d").unwrap();
    for p in read_dir_sorted(&d).iter().chain([&rlib, &d]) {
        set_mtime(p, t);
    }
}

fn incr(profile: &Path, dir: &str, n: usize, t: f64) {
    for i in 0..n {
        let s = profile.join("incremental").join(dir).join(format!("s-{i}"));
        fs::create_dir_all(&s).unwrap();
        fs::write(s.join("dep-graph.bin"), vec![b'1'; 5000]).unwrap();
        fs::write(s.with_file_name(format!("s-{i}.lock")), "").unwrap();
        set_mtime(&s, t + i as f64);
    }
}

struct Fx {
    root: PathBuf,
    p: PathBuf,
}
impl Drop for Fx {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// run 1: a, b1, app1. run 2: b2, app2. run 3: app3 (on b2) with 3 incremental sessions.
fn fixture() -> Fx {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tmp")
        .join(format!("t{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
    let p = root.join("target/debug");
    unit(&p, "a", "aaaaaaaaaaaaaaaa", 1, &[], BASE);
    unit(&p, "b", "b1b1b1b1b1b1b1b1", 2, &[("a", 1)], BASE + 1.0);
    unit(&p, "app", "0000000000000001", 3, &[("b", 2)], BASE + 2.0);
    unit(&p, "b", "b2b2b2b2b2b2b2b2", 4, &[("a", 1)], BASE + 1000.0);
    unit(&p, "app", "0000000000000002", 5, &[("b", 4)], BASE + 1001.0);
    unit(&p, "app", "0000000000000003", 6, &[("b", 4)], BASE + 2000.0);
    incr(&p, "app-0000000000000003", 3, BASE + 2000.0);
    Fx { root, p }
}

fn opts(fx: &Fx, keep: usize, apply: bool) -> Opts {
    Opts { roots: vec![fx.root.clone()], keep, gap: 60.0, apply, verbose: false }
}

fn all_files(root: &Path) -> Vec<PathBuf> {
    let mut v = vec![];
    for e in read_dir_sorted(root) {
        v.push(e.clone());
        if e.is_dir() {
            v.extend(all_files(&e));
        }
    }
    v
}

fn fp(fx: &Fx, n: &str) -> bool {
    fx.p.join(".fingerprint").join(n).exists()
}

#[test]
fn runs_grouped_by_gap() {
    let fx = fixture();
    assert_eq!(group_runs(&load_units(&fx.p), 60.0, &[]).len(), 3);
}

#[test]
fn markers_split_runs_closer_than_gap() {
    let fx = fixture();
    // a marker between b1 (BASE+1) and app1 (BASE+2) splits run 1; after the first marker
    // only markers split, so the rest is one run
    assert_eq!(group_runs(&load_units(&fx.p), 60.0, &[BASE + 1.5]).len(), 2);
    fs::write(fx.root.join("target").join(MARKERS), format!("{}\n", BASE + 1.5)).unwrap();
    assert_eq!(load_markers(&fx.p), vec![BASE + 1.5]);
}

#[test]
fn markers_override_gap_within_a_run() {
    let fx = fixture();
    // one marker before everything: the 1000 s gaps no longer split, all is one run
    assert_eq!(group_runs(&load_units(&fx.p), 60.0, &[BASE - 1.0]).len(), 1);
    // markers before run 1 and run 3: run 1+2 merge (no marker between them), run 3 separate
    assert_eq!(group_runs(&load_units(&fx.p), 60.0, &[BASE - 1.0, BASE + 1500.0]).len(), 2);
}

#[test]
fn hex_both_orders() {
    let v = hex_to_u64s("0100000000000000");
    assert!(v.contains(&1) && v.contains(&(1u64 << 56)));
    assert!(hex_to_u64s("zz").is_empty());
}

#[test]
fn dry_run_deletes_nothing() {
    let fx = fixture();
    let before = all_files(&fx.root);
    let (_, free) = prune(&opts(&fx, 1, false));
    assert!(free > 0);
    assert_eq!(before, all_files(&fx.root));
}

#[test]
fn keep_1_keeps_transitive_old_deps() {
    let fx = fixture();
    prune(&opts(&fx, 1, true));
    assert!(fp(&fx, "app-0000000000000003"));
    assert!(fp(&fx, "b-b2b2b2b2b2b2b2b2"));
    assert!(fp(&fx, "a-aaaaaaaaaaaaaaaa"), "old dep still needed");
    assert!(!fp(&fx, "b-b1b1b1b1b1b1b1b1"));
    assert!(!fp(&fx, "app-0000000000000001"));
    assert!(!fp(&fx, "app-0000000000000002"));
    assert!(!fx.p.join("deps/libb-b1b1b1b1b1b1b1b1.rlib").exists());
    assert!(!fx.p.join("deps/b-b1b1b1b1b1b1b1b1.d").exists());
    assert!(fx.p.join("deps/liba-aaaaaaaaaaaaaaaa.rlib").exists());
}

#[test]
fn keep_2_keeps_two_runs() {
    let fx = fixture();
    prune(&opts(&fx, 2, true));
    assert!(fp(&fx, "app-0000000000000002"));
    assert!(!fp(&fx, "app-0000000000000001"));
}

#[test]
fn stale_incremental_sessions_removed() {
    let fx = fixture();
    prune(&opts(&fx, 1, true));
    let inc = fx.p.join("incremental/app-0000000000000003");
    assert!(inc.join("s-2").exists() && inc.join("s-2.lock").exists());
    assert!(!inc.join("s-0").exists() && !inc.join("s-0.lock").exists());
}

#[test]
fn idle_unmatched_incremental_dir_removed() {
    let fx = fixture();
    incr(&fx.p, "app-zzzzoldcrateid", 2, BASE + 2.0); // rustc id, old run
    incr(&fx.p, "app-zzzznewcrateid", 2, BASE + 2000.0); // rustc id, current run
    prune(&opts(&fx, 1, true));
    assert!(!fx.p.join("incremental/app-zzzzoldcrateid").exists());
    let new = fx.p.join("incremental/app-zzzznewcrateid");
    assert!(new.join("s-1").exists() && !new.join("s-0").exists());
}

#[test]
fn unresolved_dep_is_fail_safe() {
    let fx = fixture();
    unit(&fx.p, "x", "9999999999999999", 7, &[("ghost", 42)], BASE + 2001.0);
    let before = all_files(&fx.root);
    let (_, free) = prune(&opts(&fx, 1, true));
    assert_eq!(free, 0);
    assert_eq!(before, all_files(&fx.root));
}

#[test]
fn rebuilt_dep_falls_back_to_name() {
    let fx = fixture();
    // app4 points at a b fingerprint that no longer exists: keep every b rather than guess
    unit(&fx.p, "app", "0000000000000004", 8, &[("b", 777)], BASE + 3000.0);
    prune(&opts(&fx, 1, true));
    assert!(fp(&fx, "b-b1b1b1b1b1b1b1b1") && fp(&fx, "b-b2b2b2b2b2b2b2b2"));
}

#[test]
fn keep_0_and_empty_dir() {
    let fx = fixture();
    prune(&opts(&fx, 0, true));
    assert!(fs::read_dir(fx.p.join(".fingerprint")).unwrap().next().is_none());
    assert_eq!(find_profile_dirs(&[fx.root.join("nope")]).len(), 0);
}
