//! Importer from rtok's index schema (`symbols` + `file_rank`). Behind the `rtok-import`
//! feature: it hardcodes rtok's table layout and is not needed to use the store.

use crate::store::{Result, SCHEMA, build_base, ensure_root_fts, fts_table, persist_base};
use rusqlite::Connection;
use std::path::Path;

fn hex_or_raw(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    if b.len().is_multiple_of(2) && b.iter().all(|c| c.is_ascii_hexdigit()) {
        (0..b.len() / 2)
            .map(|i| u8::from_str_radix(&s[2 * i..2 * i + 2], 16).unwrap())
            .collect()
    } else {
        b.to_vec()
    }
}

/// Import an rtok database (`symbols` + `file_rank`) into a fresh normalized database at `dst`.
pub fn import_rtok(src: &Path, dst: &Path) -> Result<()> {
    for ext in ["", "-wal", "-shm"] {
        let _ = std::fs::remove_file(format!("{}{ext}", dst.display()));
    }
    let conn = Connection::open(dst)?;
    conn.execute_batch("PRAGMA page_size=4096; PRAGMA journal_mode=OFF; PRAGMA synchronous=OFF; PRAGMA temp_store=MEMORY; PRAGMA cache_size=-262144;")?;
    conn.execute_batch(SCHEMA)?;
    conn.create_scalar_function(
        "hexblob",
        1,
        rusqlite::functions::FunctionFlags::SQLITE_DETERMINISTIC,
        |ctx| {
            let s: String = ctx.get(0)?;
            Ok(if s.is_empty() {
                None
            } else {
                Some(hex_or_raw(&s))
            })
        },
    )?;
    conn.execute(
        "ATTACH DATABASE ?1 AS o",
        [format!("file:{}?mode=ro", src.display())],
    )
    .or_else(|_| conn.execute("ATTACH DATABASE ?1 AS o", [src.to_string_lossy()]))?;
    conn.execute_batch(
        r#"BEGIN;
        INSERT INTO roots(path) SELECT DISTINCT root FROM o.symbols ORDER BY root;
        INSERT INTO files(root_id, path, sha, mtime, size, gen)
          SELECT r.id, s.path, coalesce(hexblob(max(s.file_sha)), x''), max(s.mtime), max(s.size), 0
          FROM o.symbols s JOIN roots r ON r.path = s.root GROUP BY r.id, s.path ORDER BY r.id, s.path;
        INSERT INTO names(id, name) VALUES (0, '');
        INSERT OR IGNORE INTO names(name) SELECT n FROM (SELECT name AS n FROM o.symbols UNION SELECT scope FROM o.symbols) ORDER BY n;
        INSERT INTO kinds(kind) SELECT DISTINCT kind FROM o.symbols ORDER BY kind;
        CREATE TEMP TABLE fmap AS SELECT f.id AS fid, r.path AS root, f.path AS path FROM files f JOIN roots r ON r.id=f.root_id;
        CREATE UNIQUE INDEX temp.fmap_i ON fmap(root, path);
        INSERT INTO defs(id, file_id, name_id, kind, line, end_line, start_byte, end_byte, content_hash, signature, doc)
          SELECT s.id, m.fid, n.id, k.id, s.line, s.end_line, s.start_byte, s.end_byte, hexblob(s.content_hash),
                 nullif(s.signature, ''), nullif(s.doc, '')
          FROM o.symbols s JOIN fmap m ON m.root=s.root AND m.path=s.path JOIN names n ON n.name=s.name JOIN kinds k ON k.kind=s.kind
          WHERE s.is_def=1 ORDER BY m.fid, s.line;
        INSERT INTO refs(file_id, name_id, scope_id, kind, line, start_byte, end_byte, content_hash)
          SELECT m.fid, n.id, sc.id, k.id, s.line, s.start_byte, s.end_byte, hexblob(s.content_hash)
          FROM o.symbols s JOIN fmap m ON m.root=s.root AND m.path=s.path JOIN names n ON n.name=s.name
               JOIN names sc ON sc.name=s.scope JOIN kinds k ON k.kind=s.kind
          WHERE s.is_def=0 ORDER BY m.fid, s.line;
        INSERT INTO file_rank(root_id, graph) SELECT r.id, fr.graph FROM o.file_rank fr JOIN roots r ON r.path=fr.root;
        INSERT OR REPLACE INTO meta VALUES('gen', 0);
        COMMIT;"#,
    )?;
    conn.execute_batch("DETACH DATABASE o;")?;
    let roots: Vec<i64> = conn
        .prepare("SELECT id FROM roots")?
        .query_map([], |r| r.get(0))?
        .collect::<Result<_>>()?;
    for &rid in &roots {
        ensure_root_fts(&conn, rid as u32)?;
        let t = fts_table(rid);
        conn.execute_batch(&format!(
            "INSERT INTO {t}({t}) VALUES('rebuild'); INSERT INTO {t}({t}) VALUES('optimize');"
        ))?;
    }
    for rid in roots {
        let base = build_base(&conn, rid as u32)?;
        persist_base(&conn, rid as u32, &base)?;
    }
    conn.execute_batch("ANALYZE; VACUUM; PRAGMA journal_mode=WAL;")?;
    Ok(())
}
