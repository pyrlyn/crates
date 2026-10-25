#![allow(dead_code)]
use sqlite_csr_graph::Connection;
use std::collections::BTreeSet;

/// Every edge of a snapshot: (users, callees, filedefs).
pub type Edges = (
    BTreeSet<(u32, u32, u8, u32)>,
    BTreeSet<(u32, u32, u32)>,
    BTreeSet<(u32, u32)>,
);

/// Set-of-(node) comparison for an edge-graph built straight from SQL.
pub fn base_edges(conn: &Connection, root: u32) -> Edges {
    let b = sqlite_csr_graph::store::build_base(conn, root).unwrap();
    dump(&sqlite_csr_graph::graph::Snapshot {
        base: b.into(),
        delta: Default::default(),
    })
}

pub fn dump(s: &sqlite_csr_graph::graph::Snapshot) -> Edges {
    let mut u = BTreeSet::new();
    let mut c = BTreeSet::new();
    let mut f = BTreeSet::new();
    let keys = |csr: &sqlite_csr_graph::segment::Csr| {
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
