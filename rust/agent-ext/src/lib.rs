// Copyright (c) 2026 Ivan Tugay
// SPDX-License-Identifier: GPL-3.0-or-later OR LicenseRef-Royalty-Free
// Licensed under GPL-3.0 or later, or under the royalty-free licence in LICENSE-ROYALTY-FREE.md

//! Extension points an agent host loads from files the user or a repository
//! wrote: `SKILL.md` skills and Claude-Code-style shell hooks. Both are
//! untrusted input and both fail open: a broken skill or hook is reported
//! (a notice, a `Failed` outcome) and skipped, never fatal. Separate from
//! any one host so cox and aulo load them the same way.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

pub mod frontmatter;
pub mod hooks;
pub mod skills;
