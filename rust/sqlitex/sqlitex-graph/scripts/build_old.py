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
# Column names cannot be bound as parameters: allow only rtok's known symbols columns, each a plain
# identifier, and double-quote them. Root values are bound via ? placeholders.
import re
ALLOWED_COLS = {"id", "path", "name", "kind", "line", "is_def", "file_sha", "root", "mtime", "size", "end_line", "scope", "start_byte", "end_byte", "content_hash", "signature", "doc"}
cols = [r[1] for r in db.execute("pragma table_info(symbols)")]
bad = [x for x in cols if x not in ALLOWED_COLS or not re.fullmatch(r"[a-z_][a-z0-9_]*", x)]
if bad: raise SystemExit(f"unexpected symbols columns: {bad}")
c = ",".join(f'"{x}"' for x in cols)
INSERT_SYMBOLS = f"insert into symbols({c}) select {c} from r.symbols where root in ({q})"
INSERT_RANK = f"insert into file_rank select * from r.file_rank where root in ({q})"
db.execute(INSERT_SYMBOLS, ROOTS)  # nosemgrep: python.sqlalchemy.security.sqlalchemy-execute-raw-query.sqlalchemy-execute-raw-query -- allowlisted quoted columns + ? placeholders only
db.execute(INSERT_RANK, ROOTS)  # nosemgrep: python.sqlalchemy.security.sqlalchemy-execute-raw-query.sqlalchemy-execute-raw-query -- only ? placeholders are interpolated
db.commit(); db.execute("detach r")
db.execute("vacuum"); db.execute("analyze") if False else None
print(db.execute("select root,count(*),sum(is_def) from symbols group by root").fetchall())
print(db.execute("select count(*) from symbols_fts").fetchone())
