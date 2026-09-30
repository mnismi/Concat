// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Everything the window's .slint tree exports.
//!
//! The Slint compiler turns `crates/concat/ui` into one enormous generated
//! module. It lives here rather than in the window crate so that the window's
//! own Rust rebuilds without the compiler checking it again; this crate
//! rebuilds only when a .slint file changes.

// The generated accessors are thousands of public items nobody documents.
#![allow(missing_docs)]

slint::include_modules!();
