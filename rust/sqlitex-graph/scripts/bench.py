"""Latency + size benchmark: rtok's current graph (data/old.db, rtok query shapes and impact BFS,
same harness as profile_old.py) vs sqlitex-graph (data/new.db + loadable extension).
Usage: python3 scripts/bench.py <path to loadable dylib>  -> prints a report, writes data/bench.json"""
import sqlite3, time, random, statistics, json, sys, os
sys.path.insert(0, os.path.dirname(__file__))
EXT = sys.argv[1]
ROOT = "/Users/listepo/GitHub/listepo/apps/rtok"
CHUNK = 500

def old_conn():
    c = sqlite3.connect("file:data/old.db?mode=ro", uri=True, check_same_thread=False)
    return c  # rtok defaults: 2 MB cache, no mmap (see profile_old.txt)

def new_conn():
    c = sqlite3.connect("data/new.db", check_same_thread=False)
    c.enable_load_extension(True); c.load_extension(EXT, entrypoint="sqlite3_extension_init"); c.enable_load_extension(False)
    for p in ("journal_mode=WAL", "synchronous=NORMAL", "cache_size=-65536", "mmap_size=268435456", "temp_store=MEMORY"):
        c.execute("pragma " + p)
    return c

OLD = {
 "defs": "SELECT path,kind,line,end_line FROM symbols WHERE root=? AND name=? AND is_def=1 ORDER BY path,line",
 "refs": "SELECT path,line FROM symbols WHERE root=? AND name=? AND is_def=0 AND kind!='import' ORDER BY path,line",
 "callers": "SELECT path,scope,count(*),min(line) FROM symbols WHERE root=? AND name=? AND is_def=0 AND kind!='import' GROUP BY path,scope ORDER BY path,scope",
 "callees": "SELECT d.path,d.line,r.name,r.line FROM symbols d JOIN symbols r ON r.root=d.root AND r.path=d.path AND r.is_def=0 AND r.scope=d.name WHERE d.root=? AND d.is_def=1 AND d.name=? AND r.name!=''",
 "outline": "SELECT name,kind,line,end_line FROM symbols WHERE root=? AND path=? AND is_def=1 ORDER BY line",
}
NEW = {  # identical to src/store.rs `sql` module
 "defs": "SELECT f.path, k.kind, d.line, d.end_line FROM defs d JOIN files f ON f.id=d.file_id JOIN kinds k ON k.id=d.kind WHERE d.name_id=(SELECT id FROM names WHERE name=?2) AND f.root_id=?1 ORDER BY f.path, d.line",
 "refs": "SELECT f.path, r.line FROM refs r JOIN files f ON f.id=r.file_id WHERE r.name_id=(SELECT id FROM names WHERE name=?2) AND f.root_id=?1 AND r.kind<>(SELECT id FROM kinds WHERE kind='import') ORDER BY f.path, r.line",
 "callers": "SELECT f.path, s.name, count(*), min(r.line) FROM refs r JOIN files f ON f.id=r.file_id JOIN names s ON s.id=r.scope_id WHERE r.name_id=(SELECT id FROM names WHERE name=?2) AND f.root_id=?1 AND r.kind<>(SELECT id FROM kinds WHERE kind='import') GROUP BY f.path, s.name ORDER BY f.path, s.name",
 "callees": "SELECT f.path, d.line, n.name, r.line FROM defs d JOIN files f ON f.id=d.file_id JOIN refs r ON r.file_id=d.file_id AND r.scope_id=d.name_id JOIN names n ON n.id=r.name_id WHERE d.name_id=(SELECT id FROM names WHERE name=?2) AND f.root_id=?1 AND r.name_id<>0",
 "outline": "SELECT n.name, k.kind, d.line, d.end_line FROM defs d JOIN names n ON n.id=d.name_id JOIN kinds k ON k.id=d.kind WHERE d.file_id=(SELECT id FROM files WHERE root_id=?1 AND path=?2) ORDER BY d.line",
 "callers_native": "SELECT path, scope FROM graph_callers(?2, ?1)",
 "callees_native": "SELECT path, callee FROM graph_callees(?2, ?1)",
}
FTS_OLD = "SELECT s.path,s.name,s.kind,s.line,s.signature,s.doc,bm25(symbols_fts) FROM symbols_fts JOIN symbols s ON s.id=symbols_fts.rowid WHERE symbols_fts MATCH ? AND s.root=? AND s.is_def=1 LIMIT 200"
FTS_NEW = "SELECT f.path, n.name, k.kind, d.line, d.signature, d.doc, bm25({t}) FROM {t} JOIN defs d ON d.id={t}.rowid JOIN files f ON f.id=d.file_id JOIN names n ON n.id=d.name_id JOIN kinds k ON k.id=d.kind WHERE {t} MATCH ? LIMIT 200"  # per-root table

def impact_refs(n): return f"SELECT name,path,scope FROM symbols WHERE root=? AND name IN ({','.join('?'*n)}) AND is_def=0 AND name!='' AND kind!='import'"
def impact_imp(n): return f"SELECT i.name,d.path,d.name FROM symbols i JOIN symbols d ON d.root=i.root AND d.path=i.path AND d.is_def=1 AND d.name!='' WHERE i.root=? AND i.name IN ({','.join('?'*n)}) AND i.kind='import' AND i.is_def=0"
def old_impact(c, name, depth):  # verbatim from profile_old.py (rtok's algorithm)
    results = {}; cur = [(name, frozenset())]; level = 1
    while cur and level <= depth:
        tips = list({t for t, _ in cur}); by = {}
        for ch in [tips[i:i+CHUNK] for i in range(0, len(tips), CHUNK)]:
            for sql in (impact_refs(len(ch)), impact_imp(len(ch))):
                for tip, p, o in c.execute(sql, [ROOT, *ch]):
                    v = by.setdefault(tip, [])
                    if (p, o) not in v: v.append((p, o))
        nxt = []
        for tip, seen in cur:
            for p, o in by.get(tip, []):
                if o in seen: continue
                results.setdefault((p, o), level)
                if o: nxt.append((o, seen | {o}))
        cur = nxt; level += 1
    return results

def pct(ts):
    ts = sorted(ts); return {"p50": statistics.median(ts), "p95": ts[max(0, int(len(ts)*.95)-1)], "max": ts[-1], "n": len(ts)}
def timeit(f, args, n=500, cap=60.0):
    ts = []; t0 = time.perf_counter(); i = 0
    while i < n and time.perf_counter() - t0 < cap:
        a = args[i % len(args)]; t = time.perf_counter(); f(a); ts.append((time.perf_counter() - t) * 1e3); i += 1
    return pct(ts)

oc, nc = old_conn(), new_conn()
rid = nc.execute("SELECT id FROM roots WHERE path=?", [ROOT]).fetchone()[0]
random.seed(7)
defs = [r[0] for r in oc.execute("SELECT DISTINCT name FROM symbols WHERE root=? AND is_def=1 AND name!=''", [ROOT])]
names = random.sample(defs, 200)
top = [r[0] for r in oc.execute("SELECT name FROM symbols WHERE root=? AND is_def=0 GROUP BY name ORDER BY count(*) DESC LIMIT 20", [ROOT])]
paths = [r[0] for r in oc.execute("SELECT DISTINCT path FROM symbols WHERE root=? LIMIT 200", [ROOT])]
# warm-up + cold start of the native graph (first TVF call loads segments)
t = time.perf_counter(); nc.execute("SELECT count(*) FROM graph_callers('x', ?)", [ROOT]).fetchall(); cold = (time.perf_counter() - t) * 1e3
R = {"graph_cold_load_ms": cold, "lat": {}}
for k in ("defs", "refs", "callers", "callees", "outline"):
    arg = paths if k == "outline" else names
    R["lat"][k] = {"old": timeit(lambda a: oc.execute(OLD[k], [ROOT, a]).fetchall(), arg),
                   "new": timeit(lambda a: nc.execute(NEW[k], [rid, a]).fetchall(), arg)}
for k in ("callers_native", "callees_native"):
    R["lat"][k] = {"new": timeit(lambda a: nc.execute(NEW[k], [ROOT, a]).fetchall(), names)}
mismatch = 0
for depth in (1, 2, 3):
    for label, pool in (("random", names[:50]), ("top", top)):
        o = timeit(lambda a: old_impact(oc, a, depth), pool, n=len(pool), cap=60)
        nn = timeit(lambda a: nc.execute("SELECT depth,path,scope FROM graph_impact(?,?,?)", [a, depth, ROOT]).fetchall(), pool, n=len(pool), cap=60)
        R["lat"][f"impact d{depth} {label}"] = {"old": o, "new": nn}
        # result check on the names the old side finished: every rtok pair present at <= depth
        for a in pool[:o["n"]] if depth < 3 else pool[:min(o["n"], 3)]:
            want = old_impact(oc, a, depth)
            got = {(p, s): d for d, p, s in nc.execute("SELECT depth,path,scope FROM graph_impact(?,?,?)", [a, depth, ROOT])}
            if any(k not in got or got[k] > v for k, v in want.items()): mismatch += 1
R["impact_mismatch"] = mismatch
for q in ("graph", "symbol OR impact", "cache"):
    R["lat"][f"fts '{q}'"] = {"old": timeit(lambda a: oc.execute(FTS_OLD, [q, ROOT]).fetchall(), [0], 200),
                              "new": timeit(lambda a: nc.execute(FTS_NEW.format(t=f"defs_fts_{rid}"), [q]).fetchall(), [0], 200)}
# sizes
def fsize(p): return sum(os.path.getsize(p + s) for s in ("", "-wal") if os.path.exists(p + s))
sym = oc.execute("SELECT count(*) FROM symbols").fetchone()[0]
refs_old = oc.execute("SELECT count(*) FROM symbols WHERE is_def=0").fetchone()[0]
stats = json.loads(nc.execute("SELECT graph_stats()").fetchone()[0])
edges = sum(s["users_edges"] + s["callee_edges"] for s in stats); seg = sum(s["base_bytes"] for s in stats)
def tbl(c, like):
    return {r[0]: r[1] for r in c.execute("SELECT name, sum(pgsize) FROM dbstat GROUP BY name")} if like else None
R["size"] = {"old_bytes": fsize("data/old.db"), "new_bytes": fsize("data/new.db"), "symbols": sym, "ref_rows": refs_old,
             "graph_edges": edges, "segment_bytes": seg, "old_tables": tbl(oc, 1), "new_tables": tbl(nc, 1)}
json.dump(R, open("data/bench.json", "w"), indent=1)
print(f"cold graph load (first TVF call): {cold:.1f} ms; impact result mismatches: {mismatch}")
print(f"{'query':28} {'old p50':>9} {'p95':>9} {'max':>9} | {'new p50':>9} {'p95':>9} {'max':>9}  (ms)")
for k, v in R["lat"].items():
    o = v.get("old"); n = v["new"]
    os_ = f"{o['p50']:9.3f} {o['p95']:9.3f} {o['max']:9.3f}" + (f" n={o['n']}" if o['n'] < n['n'] else "") if o else " " * 29
    print(f"{k:28} {os_} | {n['p50']:9.3f} {n['p95']:9.3f} {n['max']:9.3f}")
s = R["size"]
print(f"\nold.db {s['old_bytes']/1e6:.1f} MB = {s['old_bytes']/s['symbols']:.0f} B/symbol; new.db {s['new_bytes']/1e6:.1f} MB = {s['new_bytes']/s['symbols']:.0f} B/symbol")
print(f"graph: {edges} edges in {seg} B of CSR segments = {seg/edges:.2f} B/edge")
