//! Equivalence against rtok's schema and semantics on a real rtok index.
//! Needs the `rtok-import` feature and a copy of rtok's database (`RTOK_OLD_DB`, default
//! `data/old.db`), so it is ignored by default:
//! `RTOK_OLD_DB=... cargo test -p sqlite-csr-graph --features rtok-import --test equivalence -- --ignored`.
mod common;
use common::*;
use sqlite_csr_graph::store::{fts_table, sql};
use sqlite_csr_graph::{Connection, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

fn rows(c: &Connection, q: &str, p: &[&dyn sqlite_csr_graph::sqlite::ToSql]) -> Vec<Vec<Value>> {
    let mut st = c.prepare_cached(q).unwrap();
    let n = st.column_count();
    st.query_map(p, |r| (0..n).map(|i| r.get::<_, Value>(i)).collect())
        .unwrap()
        .map(|r| r.unwrap())
        .collect()
}
fn sorted(mut v: Vec<Vec<Value>>) -> Vec<String> {
    let mut s: Vec<String> = v.drain(..).map(|r| format!("{r:?}")).collect();
    s.sort();
    s
}

/// rtok's impact, verbatim port (per-path visited sets, no frontier dedup).
fn old_impact(c: &Connection, name: &str, depth: u32) -> BTreeSet<(u32, String, String)> {
    let mut results: BTreeMap<(String, String), u32> = BTreeMap::new();
    let mut cur: Vec<(String, HashSet<String>)> = vec![(name.to_string(), HashSet::new())];
    let mut level = 1;
    while !cur.is_empty() && level <= depth {
        let tips: HashSet<String> = cur.iter().map(|t| t.0.clone()).collect();
        let mut by: HashMap<String, Vec<(String, String)>> = HashMap::new();
        for tip in &tips {
            let a = rows(
                c,
                "SELECT name,path,scope FROM symbols WHERE root=?1 AND name=?2 AND is_def=0 AND name!='' AND kind!='import'",
                &[&root(), tip],
            );
            let b = rows(
                c,
                "SELECT i.name,d.path,d.name FROM symbols i JOIN symbols d ON d.root=i.root AND d.path=i.path AND d.is_def=1 AND d.name!='' WHERE i.root=?1 AND i.name=?2 AND i.kind='import' AND i.is_def=0",
                &[&root(), tip],
            );
            for r in a.into_iter().chain(b) {
                let (Value::Text(p), Value::Text(o)) = (&r[1], &r[2]) else {
                    panic!()
                };
                let v = by.entry(tip.clone()).or_default();
                if !v.contains(&(p.clone(), o.clone())) {
                    v.push((p.clone(), o.clone()));
                }
            }
        }
        let mut nxt = vec![];
        for (tip, seen) in &cur {
            for (p, o) in by.get(tip).into_iter().flatten() {
                if seen.contains(o) {
                    continue;
                }
                results.entry((p.clone(), o.clone())).or_insert(level);
                if !o.is_empty() {
                    let mut s = seen.clone();
                    s.insert(o.clone());
                    nxt.push((o.clone(), s));
                }
            }
        }
        cur = nxt;
        level += 1;
    }
    results.into_iter().map(|((p, o), d)| (d, p, o)).collect()
}

#[test]
#[ignore = "needs a real rtok index (RTOK_OLD_DB) and --features rtok-import"]
fn equivalent_to_rtok() {
    let Some(old_path) = old_db() else {
        eprintln!("data/old.db missing; skipped");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let new_path = dir.path().join("new.db");
    sqlite_csr_graph::import_rtok(&old_path, &new_path).unwrap();
    let old = Connection::open_with_flags(
        &old_path,
        sqlite_csr_graph::sqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let store = sqlite_csr_graph::Store::open(&new_path).unwrap();
    let new = store.reader().unwrap();
    let rid: i64 = new
        .query_row("SELECT id FROM roots WHERE path=?1", [root()], |r| r.get(0))
        .unwrap();

    // deterministic sample: every 40th distinct def name + most-referenced names
    let mut names: Vec<String> = rows(
        &old,
        "SELECT DISTINCT name FROM symbols WHERE root=?1 AND is_def=1 AND name!='' ORDER BY name",
        &[&root()],
    )
    .into_iter()
    .step_by(40)
    .map(|r| match &r[0] {
        Value::Text(s) => s.clone(),
        _ => unreachable!(),
    })
    .collect();
    let top: Vec<String> = rows(&old, "SELECT name FROM symbols WHERE root=?1 AND is_def=0 GROUP BY name ORDER BY count(*) DESC LIMIT 20", &[&root()])
        .into_iter().map(|r| match &r[0] { Value::Text(s) => s.clone(), _ => unreachable!() }).collect();
    names.extend(top.iter().cloned());
    let paths: Vec<String> = rows(
        &old,
        "SELECT DISTINCT path FROM symbols WHERE root=?1 ORDER BY path",
        &[&root()],
    )
    .into_iter()
    .step_by(5)
    .map(|r| match &r[0] {
        Value::Text(s) => s.clone(),
        _ => unreachable!(),
    })
    .collect();

    let o_defs =
        "SELECT path,kind,line,end_line FROM symbols WHERE root=?1 AND name=?2 AND is_def=1";
    let o_refs =
        "SELECT path,line FROM symbols WHERE root=?1 AND name=?2 AND is_def=0 AND kind!='import'";
    let o_groups = "SELECT path,scope,count(*),min(line) FROM symbols WHERE root=?1 AND name=?2 AND is_def=0 AND kind!='import' GROUP BY path,scope";
    let o_callees = "SELECT d.path,d.line,r.name,r.line FROM symbols d JOIN symbols r ON r.root=d.root AND r.path=d.path AND r.is_def=0 AND r.scope=d.name WHERE d.root=?1 AND d.is_def=1 AND d.name=?2 AND r.name!=''";
    let o_file =
        "SELECT name,kind,line,end_line FROM symbols WHERE root=?1 AND path=?2 AND is_def=1";
    let mut checked = 0;
    for n in &names {
        assert_eq!(
            sorted(rows(&old, o_defs, &[&root(), n])),
            sorted(rows(&new, sql::DEFS, &[&rid, n])),
            "defs {n}"
        );
        assert_eq!(
            sorted(rows(&old, o_refs, &[&root(), n])),
            sorted(rows(&new, sql::REFS, &[&rid, n])),
            "refs {n}"
        );
        assert_eq!(
            sorted(rows(&old, o_groups, &[&root(), n])),
            sorted(rows(&new, sql::REF_GROUPS, &[&rid, n])),
            "groups {n}"
        );
        let oc = rows(&old, o_callees, &[&root(), n]);
        assert_eq!(
            sorted(oc.clone()),
            sorted(rows(&new, sql::CALLEES, &[&rid, n])),
            "callees {n}"
        );
        // native callers/callees == distinct projections of rtok's rows
        let og: BTreeSet<String> = rows(&old, o_groups, &[&root(), n])
            .into_iter()
            .map(|r| format!("{:?}", &r[..2]))
            .collect();
        let ng: BTreeSet<String> = rows(
            &new,
            "SELECT path, scope FROM graph_callers(?1, ?2)",
            &[n, &root()],
        )
        .into_iter()
        .map(|r| format!("{r:?}"))
        .collect();
        assert_eq!(og, ng, "graph_callers {n}");
        let oce: BTreeSet<String> = oc
            .into_iter()
            .map(|r| format!("{:?}", [r[0].clone(), r[2].clone()]))
            .collect();
        let nce: BTreeSet<String> = rows(
            &new,
            "SELECT path, callee FROM graph_callees(?1, ?2)",
            &[n, &root()],
        )
        .into_iter()
        .map(|r| format!("{r:?}"))
        .collect();
        assert_eq!(oce, nce, "graph_callees {n}");
        checked += 1;
    }
    for p in &paths {
        assert_eq!(
            sorted(rows(&old, o_file, &[&root(), p])),
            sorted(rows(&new, sql::FILE_DEFS, &[&rid, p])),
            "outline {p}"
        );
    }
    // impact: reachable (depth, path, scope) sets must match rtok exactly
    let mut imp = 0;
    let mut extra = 0;
    for (i, n) in names.iter().enumerate() {
        let depths: &[u32] = if i >= names.len() - top.len() {
            &[1]
        } else {
            &[1, 2, 3]
        };
        for &d in depths {
            let want = old_impact(&old, n, d);
            if want.len() > 200_000 {
                continue;
            }
            let got: BTreeSet<(u32, String, String)> = rows(
                &new,
                "SELECT depth, path, scope FROM graph_impact(?1, ?2, ?3)",
                &[n, &d, &root()],
            )
            .into_iter()
            .map(|r| match (&r[0], &r[1], &r[2]) {
                (Value::Integer(a), Value::Text(b), Value::Text(c)) => {
                    (*a as u32, b.clone(), c.clone())
                }
                _ => panic!(),
            })
            .collect();
            // Semantics change (documented): rtok drops an edge whose owner is already on the
            // current path (per-path cycle guard); global BFS keeps it. So:
            // 1) every rtok (path, scope) pair is present, at the same or a smaller depth,
            // 2) the reachable symbol (owner) sets are identical,
            // 3) every extra/shallower row is a cycle-closing edge to an owner reached earlier.
            let gm: BTreeMap<(String, String), u32> = got
                .iter()
                .map(|(d, p, o)| ((p.clone(), o.clone()), *d))
                .collect();
            for (dw, p, o) in &want {
                let dg = gm
                    .get(&(p.clone(), o.clone()))
                    .unwrap_or_else(|| panic!("impact {n} depth {d}: missing {p} {o}"));
                assert!(dg <= dw, "impact {n} depth {d}: {p} {o} deeper");
            }
            let owners = |s: &BTreeSet<(u32, String, String)>| {
                s.iter().map(|r| r.2.clone()).collect::<BTreeSet<_>>()
            };
            assert_eq!(
                owners(&want),
                owners(&got),
                "impact {n} depth {d}: reachable symbols differ"
            );
            let min_owner_depth: BTreeMap<String, u32> =
                want.iter().fold(BTreeMap::new(), |mut m, (d2, _, o2)| {
                    let e = m.entry(o2.clone()).or_insert(*d2);
                    *e = (*e).min(*d2);
                    m
                });
            for (dd, _, o) in got.difference(&want) {
                assert!(
                    o == n || min_owner_depth.get(o).is_some_and(|m| m < dd),
                    "impact {n} depth {d}: extra row owner {o} not reached earlier"
                );
                extra += 1;
            }
            imp += 1;
        }
    }
    // FTS: same matching definitions as rtok's global index filtered to the root
    for q in ["graph", "symbol OR impact", "cache"] {
        let o: BTreeSet<String> = rows(&old, "SELECT s.path,s.name,s.line FROM symbols_fts JOIN symbols s ON s.id=symbols_fts.rowid WHERE symbols_fts MATCH ?1 AND s.root=?2 AND s.is_def=1", &[&q, &root()]).into_iter().map(|r| format!("{r:?}")).collect();
        let t = fts_table(rid);
        let nn: BTreeSet<String> = rows(&new, &format!("SELECT f.path, n.name, d.line FROM {t} JOIN defs d ON d.id={t}.rowid JOIN files f ON f.id=d.file_id JOIN names n ON n.id=d.name_id WHERE {t} MATCH ?1"), &[&q]).into_iter().map(|r| format!("{r:?}")).collect();
        assert_eq!(o, nn, "fts {q}");
    }
    eprintln!(
        "checked {checked} names, {} paths, {imp} impact queries ({extra} extra cycle-closing rows)",
        paths.len()
    );
}
