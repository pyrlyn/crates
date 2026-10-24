"""Build data/old.db: rtok's exact symbols/symbols_fts/file_rank schema (copied from the live
store's sqlite_master, read-only) holding only the rtok and cox roots from data/old_raw.db."""
import sqlite3, os
LIVE = os.path.expanduser("~/.rtok/rtok.db")
ROOTS = ("/Users/listepo/GitHub/listepo/apps/rtok", "/Users/listepo/GitHub/listepo/apps/cox")
out = "data/old.db"
if os.path.exists(out): os.remove(out)
live = sqlite3.connect(f"file:{LIVE}?mode=ro", uri=True)
ddl = live.execute("""select type,name,sql from sqlite_master where sql is not null and
  (tbl_name in ('symbols','file_rank') or name like 'symbols_fts%')""").fetchall()
live.close()
db = sqlite3.connect(out)
db.execute("pragma journal_mode=wal"); db.execute("pragma synchronous=normal")
order = {"table": 0, "index": 1, "trigger": 2}
for t, n, sql in sorted(ddl, key=lambda r: order.get(r[0], 9)):
    if n.startswith("symbols_fts_"):  # FTS5 shadow tables are created by the vtab
        continue
    print(t, n); db.execute(sql)
db.execute("attach 'data/old_raw.db' as r")
q = ",".join("?" * len(ROOTS))
cols = [r[1] for r in db.execute("pragma table_info(symbols)")]
c = ",".join(cols)
db.execute(f"insert into symbols({c}) select {c} from r.symbols where root in ({q})", ROOTS)
db.execute(f"insert into file_rank select * from r.file_rank where root in ({q})", ROOTS)
db.commit(); db.execute("detach r")
db.execute("vacuum"); db.execute("analyze") if False else None
print(db.execute("select root,count(*),sum(is_def) from symbols group by root").fetchall())
print(db.execute("select count(*) from symbols_fts").fetchone())
