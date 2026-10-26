//! The unrolled varint decoder and the per-encoding CSR loops: round trips at every byte
//! length boundary, malformed input, and random adjacency lists against a sorted reference.
use sqlitex_graph::segment::{
    Csr, ENC_EDGES, ENC_IDS, Edge, KIND_IMPORT, KIND_REF, get_varint, put_varint,
};

/// Deterministic xorshift generator (no `rand` dependency).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u32 {
        (self.next() % n) as u32
    }
}

#[test]
fn varints_round_trip_at_every_length_boundary() {
    let values = [
        0,
        1,
        0x7f,
        0x80,
        0x3fff,
        0x4000,
        0x1f_ffff,
        0x20_0000,
        0x0fff_ffff,
        0x1000_0000,
        u32::MAX - 1,
        u32::MAX,
    ];
    let mut buf = vec![];
    for &v in &values {
        put_varint(&mut buf, v);
    }
    let mut pos = 0;
    for &v in &values {
        assert_eq!(get_varint(&buf, &mut pos), v);
    }
    assert_eq!(pos, buf.len());
}

#[test]
fn varints_round_trip_random_values() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let values: Vec<u32> = (0..10_000)
        .map(|_| (rng.next() >> rng.below(33)) as u32)
        .collect();
    let mut buf = vec![];
    for &v in &values {
        put_varint(&mut buf, v);
    }
    let mut pos = 0;
    let got: Vec<u32> = values.iter().map(|_| get_varint(&buf, &mut pos)).collect();
    assert_eq!(got, values);
    assert_eq!(pos, buf.len());
}

#[test]
fn a_run_of_continuation_bytes_stops_after_five() {
    let buf = [0xff; 8];
    let mut pos = 0;
    get_varint(&buf, &mut pos);
    assert_eq!(pos, 5);
}

fn decode(c: &Csr, key: u32) -> Vec<Edge> {
    let mut v = vec![];
    c.for_each(key, |e| v.push(e));
    v
}

#[test]
fn random_edge_lists_decode_to_the_sorted_input() {
    let mut rng = Rng(42);
    for round in 0..50 {
        let n = rng.below(2_000) as usize;
        let mut pairs: Vec<(u32, Edge)> = (0..n)
            .map(|_| {
                let kind = if rng.below(5) == 0 {
                    KIND_IMPORT
                } else {
                    KIND_REF
                };
                let edge = Edge {
                    file: rng.below(if round % 2 == 0 { 300 } else { 5_000_000 }),
                    kind,
                    other: if kind == KIND_REF {
                        rng.below(400_000)
                    } else {
                        0
                    },
                };
                (rng.below(64), edge)
            })
            .collect();
        let c = Csr::build_edges(ENC_EDGES, pairs.clone());
        let c = Csr::from_blob(c.bytes().to_vec()).unwrap();
        pairs.sort_unstable();
        pairs.dedup();
        assert_eq!(c.len_items(), pairs.len());
        for key in 0..64 {
            let want: Vec<Edge> = pairs.iter().filter(|p| p.0 == key).map(|p| p.1).collect();
            assert_eq!(decode(&c, key), want, "round {round} key {key}");
        }
    }
}

#[test]
fn random_id_lists_decode_to_the_sorted_input() {
    let mut rng = Rng(7);
    let mut pairs: Vec<(u32, Edge)> = (0..5_000)
        .map(|_| {
            (
                rng.below(100),
                Edge {
                    file: rng.next() as u32,
                    kind: 0,
                    other: 0,
                },
            )
        })
        .collect();
    let c = Csr::build_edges(ENC_IDS, pairs.clone());
    pairs.sort_unstable();
    pairs.dedup();
    for key in 0..100 {
        let want: Vec<u32> = pairs
            .iter()
            .filter(|p| p.0 == key)
            .map(|p| p.1.file)
            .collect();
        let got: Vec<u32> = decode(&c, key).into_iter().map(|e| e.file).collect();
        assert_eq!(got, want, "key {key}");
    }
}
