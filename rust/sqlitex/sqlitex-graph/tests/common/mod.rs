#![allow(dead_code)]
use sqlitex_graph::Connection;
use sqlitex_graph::graph::{Base, Delta, Dict, FileOp, Graph, Snapshot};
use sqlitex_graph::segment::{Edge, KIND_IMPORT, KIND_REF};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

/// Deterministic xorshift generator (no `rand` dependency).
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    pub fn below(&mut self, n: u32) -> u32 {
        (self.next() % n.max(1) as u64) as u32
    }
}

fn push_ref(
    users: &mut Vec<(u32, Edge)>,
    callees: &mut Vec<(u32, Edge)>,
    rng: &mut Rng,
    file: u32,
    names: u32,
) {
    let name = 1 + rng.below(names);
    if rng.below(8) == 0 {
        users.push((
            name,
            Edge {
                file,
                kind: KIND_IMPORT,
                other: 0,
            },
        ));
        return;
    }
    let owner = if rng.below(10) == 0 {
        0
    } else {
        1 + rng.below(names)
    };
    users.push((
        name,
        Edge {
            file,
            kind: KIND_REF,
            other: owner,
        },
    ));
    if owner != 0 {
        callees.push((
            owner,
            Edge {
                file,
                kind: KIND_REF,
                other: name,
            },
        ));
    }
}

/// An in-memory graph for root `/r` (id 1) without SQLite: names `n1..=n{names}` (id 0 is
/// the empty name), files `src/f{i}.rs`, random references (some unscoped), imports and
/// definitions in the base, and a delta that replaces about one file in eight.
pub fn random_graph(seed: u64, names: u32, files: u32, refs: usize) -> Graph {
    let mut rng = Rng(seed | 1);
    let mut dict = Dict::default();
    dict.set_name(0, "");
    for i in 1..=names {
        dict.set_name(i, &format!("n{i}"));
    }
    for f in 0..files {
        dict.set_file(f, 1, &format!("src/f{f}.rs"));
    }
    dict.roots.insert("/r".into(), 1);
    let (mut users, mut callees, mut fdefs) = (vec![], vec![], vec![]);
    for _ in 0..refs {
        let file = rng.below(files);
        push_ref(&mut users, &mut callees, &mut rng, file, names);
    }
    for f in 0..files {
        for _ in 0..rng.below(4) {
            fdefs.push((f, 1 + rng.below(names)));
        }
    }
    let base = Base::from_pairs(users, callees, fdefs, 0);
    let mut ops = vec![];
    for seq in 0..files / 8 {
        let file = rng.below(files);
        let mut op = FileOp {
            seq: seq as u64 + 1,
            generation: 1,
            file,
            ..Default::default()
        };
        for _ in 0..rng.below(6) {
            push_ref(&mut op.users, &mut op.callees, &mut rng, file, names);
        }
        op.defs = (0..rng.below(3)).map(|_| 1 + rng.below(names)).collect();
        op.defs.sort_unstable();
        op.defs.dedup();
        op.users.sort_unstable();
        op.users.dedup();
        op.callees.sort_unstable();
        op.callees.dedup();
        ops.push(Arc::new(op));
    }
    let snap = Snapshot {
        base: Arc::new(base),
        delta: Arc::new(Delta::from_ops(ops)),
    };
    Graph::new(dict, HashMap::from([(1, snap)]))
}

/// `(depth, path, scope)` rows as text.
pub fn impact_text(
    g: &Graph,
    rows: &[sqlitex_graph::graph::ImpactRow],
) -> Vec<(u32, String, String)> {
    let d = g.dict.load();
    rows.iter()
        .map(|r| {
            (
                r.depth,
                d.path(r.file).to_string(),
                d.name(r.scope).to_string(),
            )
        })
        .collect()
}

/// Every edge of a snapshot: (users, callees, filedefs).
pub type Edges = (
    BTreeSet<(u32, u32, u8, u32)>,
    BTreeSet<(u32, u32, u32)>,
    BTreeSet<(u32, u32)>,
);

/// Root path inside the rtok fixture database (`RTOK_ROOT`, default: rtok's own checkout).
pub fn root() -> String {
    std::env::var("RTOK_ROOT").unwrap_or_else(|_| "/Users/listepo/GitHub/listepo/apps/rtok".into())
}

pub fn old_db() -> Option<PathBuf> {
    let p = std::env::var_os("RTOK_OLD_DB")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data/old.db"));
    p.exists().then_some(p)
}

/// Set-of-(node) comparison for an edge-graph built straight from SQL.
pub fn base_edges(conn: &Connection, root: u32) -> Edges {
    let b = sqlitex_graph::store::build_base(conn, root).unwrap();
    dump(&sqlitex_graph::graph::Snapshot {
        base: b.into(),
        delta: Default::default(),
    })
}

pub fn dump(s: &sqlitex_graph::graph::Snapshot) -> Edges {
    let mut u = BTreeSet::new();
    let mut c = BTreeSet::new();
    let mut f = BTreeSet::new();
    let keys = |csr: &sqlitex_graph::segment::Csr| {
        (0..csr.len_keys()).map(|i| csr.key(i)).collect::<Vec<_>>()
    };
    let mut ukeys: BTreeSet<u32> = keys(&s.base.users).into_iter().collect();
    let mut ckeys: BTreeSet<u32> = keys(&s.base.callees).into_iter().collect();
    let mut fkeys: BTreeSet<u32> = keys(&s.base.filedefs).into_iter().collect();
    for op in &s.delta.ops {
        ukeys.extend(op.users.iter().map(|x| x.0));
        ckeys.extend(op.callees.iter().map(|x| x.0));
        fkeys.insert(op.file);
    }
    for k in ukeys {
        s.users(k, |e| {
            u.insert((k, e.file, e.kind, e.other));
        });
    }
    for k in ckeys {
        s.callees(k, |e| {
            c.insert((k, e.file, e.other));
        });
    }
    for k in fkeys {
        s.filedefs(k, |n| {
            f.insert((k, n));
        });
    }
    (u, c, f)
}
