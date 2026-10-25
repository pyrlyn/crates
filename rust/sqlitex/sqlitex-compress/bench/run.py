#!/usr/bin/env python3
"""Realistic-data benchmark for sqlitex-compress (stdlib only).

1. Builds a temporary SQLite DB (users, JSON events, text posts, logs).
2. Exports its row payloads and runs `examples/dbbench.rs` once per algorithm
   (`none` = baseline, brotli, zstd, lz4): compression ratio and throughput.
3. Inserts each algorithm's output into a fresh SQLite DB: rows/s and file size.
4. Samples the driver's RSS (peak + average).
All temp data lives in one tempfile.mkdtemp() dir, removed on exit, error,
Ctrl-C or SIGTERM.

Usage: python3 bench/run.py [--rows N] [--algs none,brotli,zstd,lz4] [--keep]
"""
import argparse, json, os, random, shutil, signal, sqlite3, struct, subprocess
import sys, tempfile, threading, time

CRATE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
WORDS = ("sqlite blob compression index page cache query vector graph user "
         "event click session token request latency error warning retry").split()


def build_db(path, rows, rng):
    db = sqlite3.connect(path)
    db.executescript("""
      CREATE TABLE users(id INTEGER PRIMARY KEY, name TEXT, email TEXT, bio TEXT);
      CREATE TABLE events(id INTEGER PRIMARY KEY, user_id INT, payload TEXT);
      CREATE TABLE posts(id INTEGER PRIMARY KEY, user_id INT, body TEXT);
      CREATE TABLE logs(id INTEGER PRIMARY KEY, line TEXT);""")
    n_users = max(1, rows // 10)
    words = lambda k: " ".join(rng.choice(WORDS) for _ in range(k))
    db.executemany("INSERT INTO users VALUES(?,?,?,?)",
                   ((i, f"user{i}", f"user{i}@example.com", words(rng.randint(5, 40)))
                    for i in range(n_users)))
    db.executemany("INSERT INTO events(user_id,payload) VALUES(?,?)",
                   ((rng.randrange(n_users), json.dumps({
                       "type": rng.choice(["click", "view", "purchase", "login"]),
                       "ts": 1_760_000_000 + i, "path": f"/p/{rng.randrange(500)}",
                       "props": {w: rng.randrange(1000) for w in rng.sample(WORDS, 6)},
                   })) for i in range(rows)))
    db.executemany("INSERT INTO posts(user_id,body) VALUES(?,?)",
                   ((rng.randrange(n_users), words(rng.randint(50, 600)))
                    for _ in range(rows // 4)))
    db.executemany("INSERT INTO logs(line) VALUES(?)",
                   ((f"2026-10-25T12:{i % 60:02d}:00Z {rng.choice(['INFO','WARN','ERROR'])} "
                     f"req={rng.getrandbits(64):016x} {words(12)}",) for i in range(rows)))
    db.commit()
    return db


def export_rows(db, path):
    with open(path, "wb") as f:
        q = ("SELECT bio FROM users UNION ALL SELECT payload FROM events "
             "UNION ALL SELECT body FROM posts UNION ALL SELECT line FROM logs")
        for (v,) in db.execute(q):
            b = v.encode()
            f.write(struct.pack("<I", len(b)) + b)


def read_rows(path):
    with open(path, "rb") as f:
        buf = f.read()
    i = 0
    while i < len(buf):
        (n,) = struct.unpack_from("<I", buf, i)
        yield buf[i + 4:i + 4 + n]
        i += 4 + n


def rss_kb(pid):
    out = subprocess.run(["ps", "-o", "rss=", "-p", str(pid)],
                         capture_output=True, text=True).stdout.strip()
    return int(out) if out else 0


def run_driver(binary, env):
    """Run the driver, sampling RSS via `ps` every 10 ms; peak from getrusage."""
    p = subprocess.Popen([binary], env=env, stdout=subprocess.PIPE, text=True)
    samples, stop = [], threading.Event()

    def sample():
        while not stop.is_set():
            kb = rss_kb(p.pid)
            if kb:
                samples.append(kb)
            time.sleep(0.01)

    t = threading.Thread(target=sample, daemon=True)
    t.start()
    _, status, ru = os.wait4(p.pid, 0)
    p.returncode = os.waitstatus_to_exitcode(status)
    stop.set(); t.join()
    out = p.stdout.read()
    if p.returncode:
        raise RuntimeError(f"driver failed ({p.returncode})")
    # ru_maxrss is bytes on macOS, KiB on Linux.
    peak = ru.ru_maxrss / (1 << 20) if sys.platform == "darwin" else ru.ru_maxrss / 1024
    avg = sum(samples) / len(samples) / 1024 if samples else float("nan")
    return json.loads(out.strip().splitlines()[-1]), peak, avg, len(samples)


def insert_bench(rows_path, db_path):
    db = sqlite3.connect(db_path)
    db.execute("CREATE TABLE blobs(id INTEGER PRIMARY KEY, data BLOB)")
    rows = list(read_rows(rows_path))
    t = time.perf_counter()
    db.executemany("INSERT INTO blobs(data) VALUES(?)", ((r,) for r in rows))
    db.commit()
    dt = time.perf_counter() - t
    db.close()
    return len(rows) / dt, os.path.getsize(db_path)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--rows", type=int, default=20000, help="events/log rows (scale)")
    ap.add_argument("--algs", default="none,brotli,zstd,lz4")
    ap.add_argument("--seed", type=int, default=1)
    args = ap.parse_args()

    tmp = tempfile.mkdtemp(prefix="sqlitex-compress-bench-")

    def on_signal(sig, _frame):
        raise KeyboardInterrupt(f"signal {sig}")
    signal.signal(signal.SIGTERM, on_signal)
    signal.signal(signal.SIGINT, on_signal)
    try:
        target = os.path.join(tmp, "target")
        subprocess.run(["cargo", "build", "--release", "-q", "-p", "sqlitex-compress",
                        "--example", "dbbench", "--features", "brotli,zstd,lz4"],
                       cwd=CRATE, check=True,
                       env={**os.environ, "CARGO_TARGET_DIR":
                            os.environ.get("CARGO_TARGET_DIR", target)})
        binary = os.path.join(os.environ.get("CARGO_TARGET_DIR", target),
                              "release", "examples", "dbbench")
        src = os.path.join(tmp, "source.db")
        t = time.perf_counter()
        db = build_db(src, args.rows, random.Random(args.seed))
        rows_path = os.path.join(tmp, "rows.bin")
        export_rows(db, rows_path)
        db.close()
        print(f"source DB: {os.path.getsize(src) / 1e6:.1f} MB, built in "
              f"{time.perf_counter() - t:.1f}s ({tmp})")
        hdr = (f"{'alg':7} {'rows':>7} {'ratio':>6} {'comp MB/s':>10} {'decomp MB/s':>12} "
               f"{'insert rows/s':>14} {'db MB':>7} {'peak RSS MB':>12} {'avg RSS MB':>11}")
        print(hdr); print("-" * len(hdr))
        for alg in args.algs.split(","):
            out = os.path.join(tmp, f"{alg}.bin")
            r, peak, avg, n = run_driver(binary, {**os.environ, "SQLITEX_BENCH_INPUT": rows_path,
                                                  "SQLITEX_BENCH_OUTPUT": out,
                                                  "SQLITEX_BENCH_ALG": alg})
            ips, size = insert_bench(out, os.path.join(tmp, f"{alg}.db"))
            ratio = r["raw_bytes"] / max(1, r["compressed_bytes"])
            print(f"{alg:7} {r['rows']:>7} {ratio:>6.2f} {r['compress_mb_s']:>10.1f} "
                  f"{r['decompress_mb_s']:>12.1f} {ips:>14,.0f} {size / 1e6:>7.2f} "
                  f"{peak:>12.1f} {avg:>11.1f}")
            os.remove(out)
    except KeyboardInterrupt as e:
        print(f"interrupted ({e}), cleaning up", file=sys.stderr)
        return 130
    finally:
        signal.signal(signal.SIGINT, signal.SIG_IGN)
        shutil.rmtree(tmp, ignore_errors=True)
        print(f"removed {tmp}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
