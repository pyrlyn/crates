//! Immutable CSR segments: `keys` (sorted u32) + `offsets` (u32) + varint-packed payload.
//!
//! Blob layout (little endian): `b"RGS1" | enc u32 | nkeys u32 | nitems u32 |
//! keys[nkeys] u32 | offsets[nkeys+1] u32 | payload`.
//! Readers decode straight out of the blob (no deserialisation step), so a segment
//! can live in a SQLite BLOB, a `Vec<u8>`, or an mmap'd file.

use std::sync::Arc;

pub const MAGIC: &[u8; 4] = b"RGS1";
/// Payload encodings.
pub const ENC_EDGES: u32 = 1; // (file delta << 1 | kind) varint, then `other` varint when kind == REF
pub const ENC_IDS: u32 = 2; // sorted u32 list, delta varint

pub const KIND_REF: u8 = 0;
pub const KIND_IMPORT: u8 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Edge {
    pub file: u32,
    pub kind: u8,
    /// Owner scope (users segment), callee (callees segment); 0 for imports.
    pub other: u32,
}

#[inline]
pub fn put_varint(out: &mut Vec<u8>, mut v: u32) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

/// Decode the varint at `*pos` and advance past it. Unrolled for the one to five bytes a
/// `u32` takes, with the one-byte case (most file deltas) first; a run of continuation
/// bytes stops after the fifth byte instead of shifting past 32 bits.
#[inline]
pub fn get_varint(buf: &[u8], pos: &mut usize) -> u32 {
    let p = *pos;
    let b = buf[p];
    if b < 0x80 {
        *pos = p + 1;
        return b as u32;
    }
    let mut v = (b & 0x7f) as u32;
    let b = buf[p + 1];
    v |= ((b & 0x7f) as u32) << 7;
    if b < 0x80 {
        *pos = p + 2;
        return v;
    }
    let b = buf[p + 2];
    v |= ((b & 0x7f) as u32) << 14;
    if b < 0x80 {
        *pos = p + 3;
        return v;
    }
    let b = buf[p + 3];
    v |= ((b & 0x7f) as u32) << 21;
    if b < 0x80 {
        *pos = p + 4;
        return v;
    }
    *pos = p + 5;
    // Only the low four bits of the fifth byte fit in a u32; the shift drops the rest.
    v | ((buf[p + 4] as u32) << 28)
}

#[inline]
fn u32_at(buf: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]])
}

#[derive(Clone)]
pub struct Csr {
    blob: Arc<[u8]>,
    enc: u32,
    nkeys: usize,
    nitems: usize,
}

impl std::fmt::Debug for Csr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Csr(enc={}, keys={}, items={}, bytes={})",
            self.enc,
            self.nkeys,
            self.nitems,
            self.blob.len()
        )
    }
}

impl Csr {
    pub fn empty(enc: u32) -> Csr {
        Self::build_edges(enc, Vec::new())
    }

    /// Build from `(key, edge)` pairs. For `ENC_IDS` only `edge.file` is used (as the id).
    /// Duplicates are removed.
    pub fn build_edges(enc: u32, mut pairs: Vec<(u32, Edge)>) -> Csr {
        pairs.sort_unstable();
        pairs.dedup();
        let mut keys: Vec<u32> = Vec::new();
        let mut offsets: Vec<u32> = Vec::new();
        let mut payload: Vec<u8> = Vec::with_capacity(pairs.len() * 3);
        let mut prev_key = None;
        let mut prev = 0u32;
        for (k, e) in &pairs {
            if prev_key != Some(*k) {
                keys.push(*k);
                offsets.push(payload.len() as u32);
                prev_key = Some(*k);
                prev = 0;
            }
            match enc {
                ENC_EDGES => {
                    put_varint(&mut payload, ((e.file - prev) << 1) | e.kind as u32);
                    if e.kind == KIND_REF {
                        put_varint(&mut payload, e.other);
                    }
                    prev = e.file;
                }
                _ => {
                    put_varint(&mut payload, e.file - prev);
                    prev = e.file;
                }
            }
        }
        offsets.push(payload.len() as u32);
        let mut blob = Vec::with_capacity(16 + keys.len() * 8 + 4 + payload.len());
        blob.extend_from_slice(MAGIC);
        blob.extend_from_slice(&enc.to_le_bytes());
        blob.extend_from_slice(&(keys.len() as u32).to_le_bytes());
        blob.extend_from_slice(&(pairs.len() as u32).to_le_bytes());
        for k in &keys {
            blob.extend_from_slice(&k.to_le_bytes());
        }
        for o in &offsets {
            blob.extend_from_slice(&o.to_le_bytes());
        }
        blob.extend_from_slice(&payload);
        Csr {
            blob: blob.into(),
            enc,
            nkeys: keys.len(),
            nitems: pairs.len(),
        }
    }

    pub fn from_blob(blob: Vec<u8>) -> Result<Csr, String> {
        if blob.len() < 16 || &blob[0..4] != MAGIC {
            return Err("bad segment magic".into());
        }
        let enc = u32_at(&blob, 4);
        let nkeys = u32_at(&blob, 8) as usize;
        let nitems = u32_at(&blob, 12) as usize;
        if blob.len() < 16 + nkeys * 8 + 4 {
            return Err("truncated segment".into());
        }
        Ok(Csr {
            blob: blob.into(),
            enc,
            nkeys,
            nitems,
        })
    }

    pub fn bytes(&self) -> &[u8] {
        &self.blob
    }
    pub fn len_items(&self) -> usize {
        self.nitems
    }
    pub fn len_keys(&self) -> usize {
        self.nkeys
    }
    pub fn key(&self, i: usize) -> u32 {
        u32_at(&self.blob, 16 + i * 4)
    }
    fn offset(&self, i: usize) -> usize {
        u32_at(&self.blob, 16 + self.nkeys * 4 + i * 4) as usize
    }
    fn payload(&self) -> &[u8] {
        &self.blob[16 + self.nkeys * 8 + 4..]
    }

    /// Binary search over the key array.
    pub fn find(&self, key: u32) -> Option<usize> {
        let (mut lo, mut hi) = (0usize, self.nkeys);
        while lo < hi {
            let mid = (lo + hi) / 2;
            let k = self.key(mid);
            if k < key {
                lo = mid + 1;
            } else if k > key {
                hi = mid;
            } else {
                return Some(mid);
            }
        }
        None
    }

    /// Visit the decoded list at key index `i`. One loop per encoding (the encoding test is
    /// outside the loop), decoding from the list's own slice.
    #[inline]
    pub fn for_each_at(&self, i: usize, mut f: impl FnMut(Edge)) {
        let p = &self.payload()[self.offset(i)..self.offset(i + 1)];
        let mut pos = 0;
        let mut prev = 0u32;
        if self.enc == ENC_EDGES {
            while pos < p.len() {
                let v = get_varint(p, &mut pos);
                let kind = (v & 1) as u8;
                prev = prev.wrapping_add(v >> 1);
                let other = if kind == KIND_REF {
                    get_varint(p, &mut pos)
                } else {
                    0
                };
                f(Edge {
                    file: prev,
                    kind,
                    other,
                });
            }
        } else {
            while pos < p.len() {
                prev = prev.wrapping_add(get_varint(p, &mut pos));
                f(Edge {
                    file: prev,
                    kind: 0,
                    other: 0,
                });
            }
        }
    }

    pub fn for_each(&self, key: u32, f: impl FnMut(Edge)) {
        if let Some(i) = self.find(key) {
            self.for_each_at(i, f)
        }
    }

    pub fn contains_id(&self, key: u32, id: u32) -> bool {
        let mut hit = false;
        self.for_each(key, |e| hit |= e.file == id);
        hit
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip() {
        let pairs = vec![
            (
                5,
                Edge {
                    file: 3,
                    kind: KIND_REF,
                    other: 9,
                },
            ),
            (
                5,
                Edge {
                    file: 1,
                    kind: KIND_IMPORT,
                    other: 0,
                },
            ),
            (
                2,
                Edge {
                    file: 300_000,
                    kind: KIND_REF,
                    other: 0,
                },
            ),
            (
                5,
                Edge {
                    file: 3,
                    kind: KIND_REF,
                    other: 9,
                },
            ),
        ];
        let c = Csr::build_edges(ENC_EDGES, pairs);
        let c = Csr::from_blob(c.bytes().to_vec()).unwrap();
        let mut got = vec![];
        c.for_each(5, |e| got.push(e));
        assert_eq!(
            got,
            vec![
                Edge {
                    file: 1,
                    kind: 1,
                    other: 0
                },
                Edge {
                    file: 3,
                    kind: 0,
                    other: 9
                }
            ]
        );
        got.clear();
        c.for_each(2, |e| got.push(e));
        assert_eq!(got[0].file, 300_000);
        assert!(c.find(4).is_none());
        assert_eq!(c.len_items(), 3);
    }
}
