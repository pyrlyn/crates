//! `cargo run --example import_rtok --features rtok-import -- <rtok.db> <out.db>`: convert an rtok index into the normalized store.
use std::time::Instant;
fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() != 3 {
        eprintln!("usage: import_rtok <rtok.db> <out.db>");
        std::process::exit(2);
    }
    let t = Instant::now();
    sqlitex_graph::import_rtok(a[1].as_ref(), a[2].as_ref()).expect("import failed");
    let conn = sqlitex_graph::Connection::open(&a[2]).unwrap();
    let g = sqlitex_graph::store::load_graph(&conn).unwrap();
    println!("import_secs={:.3}", t.elapsed().as_secs_f64());
    println!("graph={}", g.stats());
}
