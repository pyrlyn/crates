//! Multi-thread read QPS: rtok's current layout (one Mutex<Connection>, or one WAL connection
//! per thread) vs sqlitex-graph (connection per thread + shared lock-free graph).
//! `cargo run --release --example qps -- data/old.db data/new.db`
use sqlitex_graph::sqlite::{Connection, OpenFlags};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn root() -> String {
    std::env::var("RTOK_ROOT").unwrap_or_else(|_| "/Users/listepo/GitHub/listepo/apps/rtok".into())
}
const SECS: f64 = 3.0;
const OLD: [&str; 5] = [
    "SELECT path,kind,line,end_line FROM symbols WHERE root=?1 AND name=?2 AND is_def=1 ORDER BY path,line",
    "SELECT path,line FROM symbols WHERE root=?1 AND name=?2 AND is_def=0 AND kind!='import' ORDER BY path,line",
    "SELECT path,scope,count(*),min(line) FROM symbols WHERE root=?1 AND name=?2 AND is_def=0 AND kind!='import' GROUP BY path,scope ORDER BY path,scope",
    "SELECT d.path,d.line,r.name,r.line FROM symbols d JOIN symbols r ON r.root=d.root AND r.path=d.path AND r.is_def=0 AND r.scope=d.name WHERE d.root=?1 AND d.is_def=1 AND d.name=?2 AND r.name!=''",
    // impact depth 1 = rtok's two statements for one tip
    "SELECT name,path,scope FROM symbols WHERE root=?1 AND name IN (?2) AND is_def=0 AND name!='' AND kind!='import' UNION ALL \
     SELECT i.name,d.path,d.name FROM symbols i JOIN symbols d ON d.root=i.root AND d.path=i.path AND d.is_def=1 AND d.name!='' WHERE i.root=?1 AND i.name IN (?2) AND i.kind='import' AND i.is_def=0",
];

fn new_sql() -> [String; 5] {
    use sqlitex_graph::store::sql;
    [
        sql::DEFS.into(),
        sql::REFS.into(),
        sql::REF_GROUPS.into(),
        sql::CALLEES.into(),
        "SELECT depth, path, scope FROM graph_impact(?2, 1, (SELECT path FROM roots WHERE id=?1))"
            .into(),
    ]
}

fn run_ops(
    c: &Connection,
    sqls: &[String],
    root: &dyn sqlitex_graph::sqlite::ToSql,
    names: &[String],
    i: &mut usize,
) -> u64 {
    let mut rows = 0;
    for q in sqls {
        let n = &names[*i % names.len()];
        *i += 1;
        let mut st = c.prepare_cached(q).unwrap();
        let mut r = st.query(sqlitex_graph::params![root, n]).unwrap();
        while r.next().unwrap().is_some() {
            rows += 1;
        }
    }
    rows
}

fn bench(
    threads: usize,
    mk: &(dyn Fn() -> Connection + Sync),
    shared: Option<Arc<Mutex<Connection>>>,
    sqls: &[String],
    root: sqlitex_graph::Value,
    names: &[String],
) -> f64 {
    let stop = AtomicBool::new(false);
    let ops = AtomicU64::new(0);
    std::thread::scope(|s| {
        for t in 0..threads {
            let (stop, ops, shared, root) = (&stop, &ops, shared.clone(), root.clone());
            s.spawn(move || {
                let own = if shared.is_none() { Some(mk()) } else { None };
                let mut i = t * 37;
                let batch = std::env::var("QPS_TXN")
                    .ok()
                    .and_then(|v| v.parse::<u64>().ok())
                    .unwrap_or(0);
                let mut in_txn = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    match &shared {
                        Some(m) => {
                            let c = m.lock().unwrap();
                            run_ops(&c, sqls, &root, names, &mut i);
                        }
                        None => {
                            let c = own.as_ref().unwrap();
                            if batch > 0 && in_txn == 0 {
                                c.execute_batch("BEGIN").unwrap();
                            }
                            run_ops(c, sqls, &root, names, &mut i);
                            if batch > 0 {
                                in_txn += 1;
                                if in_txn == batch {
                                    c.execute_batch("COMMIT").unwrap();
                                    in_txn = 0;
                                }
                            }
                        }
                    }
                    ops.fetch_add(sqls.len() as u64, Ordering::Relaxed);
                }
            });
        }
        std::thread::sleep(Duration::from_secs_f64(SECS));
        stop.store(true, Ordering::Relaxed);
    });
    ops.load(Ordering::Relaxed) as f64 / SECS
}

fn main() {
    // must run before the first connection; affects every connection in the process
    // (not in the loadable build, where the host owns SQLite)
    #[cfg(not(feature = "loadable"))]
    if std::env::var("QPS_MEMSTATUS0").is_ok() {
        assert!(sqlitex_graph::disable_sqlite_memstatus());
    }
    let a: Vec<String> = std::env::args().collect();
    let (old, new) = (a[1].clone(), a[2].clone());
    let ro = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let probe = Connection::open_with_flags(&old, ro).unwrap();
    let mut names: Vec<String> = probe.prepare("SELECT DISTINCT name FROM symbols WHERE root=?1 AND is_def=1 AND name!='' ORDER BY name").unwrap()
        .query_map([root()], |r| r.get(0)).unwrap().map(|r| r.unwrap()).collect();
    names = names.into_iter().step_by(25).collect();
    let rid: i64 = Connection::open(&new)
        .unwrap()
        .query_row("SELECT id FROM roots WHERE path=?1", [root()], |r| r.get(0))
        .unwrap();
    let old_sql: Vec<String> = OLD.iter().map(|s| s.to_string()).collect();
    let mut nsql = new_sql().to_vec();
    match std::env::var("QPS_ONLY").as_deref() {
        Ok("sql") => nsql.truncate(4),
        Ok("tvf") => nsql = vec![nsql[4].clone()],
        _ => {}
    }
    let mk_old = || Connection::open_with_flags(&old, ro).unwrap(); // rtok pragmas: default 2 MB cache, no mmap
    let t = Instant::now();
    let mk_new = || {
        let c = Connection::open_with_flags(
            &new,
            if std::env::var("QPS_RO").is_ok() {
                ro
            } else {
                OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX
            },
        )
        .unwrap();
        sqlitex_graph::store::tune(&c).unwrap();
        sqlitex_graph::register(&c).unwrap();
        if let Ok(p) = std::env::var("QPS_PRAGMA") {
            c.execute_batch(&p).unwrap();
        }
        c
    };
    let c0 = mk_new();
    c0.query_row(
        "SELECT count(*) FROM graph_callers('x', ?1)",
        [root()],
        |r| r.get::<_, i64>(0),
    )
    .unwrap();
    println!(
        "cold_open_plus_graph_load_ms={:.2}",
        t.elapsed().as_secs_f64() * 1e3
    );
    println!("threads,old_mutex_qps,old_wal_multiconn_qps,new_qps");
    for th in [1, 2, 4, 8, 16] {
        let m = Arc::new(Mutex::new(mk_old()));
        let q1 = bench(th, &mk_old, Some(m), &old_sql, root().into(), &names);
        let q2 = bench(th, &mk_old, None, &old_sql, root().into(), &names);
        let q3 = bench(th, &mk_new, None, &nsql, rid.into(), &names);
        println!("{th},{q1:.0},{q2:.0},{q3:.0}");
    }
}
