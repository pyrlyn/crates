// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later
// Licensed under GPL-3.0 or later; see https://www.gnu.org/licenses/gpl-3.0.html

//! Parameter presets per recall target, taken from the recorded benchmark
//! (`docs/benchmark.md`: 100k x 384, half uniform, half clustered, recall@10).

use crate::{Params, Quant};

/// Recall@10 target for [`Params::for_recall`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recall {
    /// About 0.90 ("prototype" grade in the benchmark).
    P90,
    /// About 0.95 ("production" grade in the benchmark).
    P95,
}

impl Params {
    /// `m = 32`, `ef_construction = 400` and an `ef_search` for `target` with `quant`.
    ///
    /// | target | `F32` | `I8` | `I8Rerank` |
    /// |---|---|---|---|
    /// | `P90` | 768 | 768 | 768 |
    /// | `P95` | 1024 | 1280 | 1024 |
    ///
    /// Measured: `ef_search` 512 gave 0.892 (`F32`) / 0.884 (`I8`), 1024 gave
    /// 0.963 / 0.951, 2048 gave 0.992 / 0.980. 768 and 1280 are interpolated, not
    /// measured. Uniform (hard) data drives these numbers; clustered, embedding-like
    /// data reached 0.986 already at 64, so lower `ef_search` with
    /// [`VecDb::set_ef_search`](crate::VecDb::set_ef_search) after measuring your own recall.
    pub fn for_recall(target: Recall, quant: Quant) -> Self {
        let ef_search = match (target, quant) {
            (Recall::P90, _) => 768,
            (Recall::P95, Quant::I8) => 1280,
            (Recall::P95, Quant::F32 | Quant::I8Rerank) => 1024,
        };
        Params {
            m: 32,
            ef_construction: 400,
            ef_search,
            quant,
            ..Params::default()
        }
    }
}
