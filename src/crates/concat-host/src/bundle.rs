// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Copying the files a saved bundle - a template or a piece - carries.

use std::path::Path;

use crate::projects;

/// Where a bundle keeps its files, relative to its folder.
pub const ASSETS: &str = "assets";

/// Copies one source file into `assets/` and returns its bundle-relative
/// path. The bundle must be self-contained, so a missing source is an error,
/// not a warning - a template that cannot find its own music is not one.
pub fn bundle_file(assets: &Path, id: &str, source: &str) -> Result<String, String> {
    if source.is_empty() {
        return Err(
            "a media item has no file behind it; fill or remove it before saving".to_owned(),
        );
    }
    let base = Path::new(source)
        .file_name()
        .and_then(|name| name.to_str())
        .map(projects::folder_name)
        .unwrap_or_else(|| "file".to_owned());
    let name = format!("{}-{base}", projects::folder_name(id));
    let destination = assets.join(&name);
    std::fs::copy(source, &destination)
        .map_err(|error| format!("could not bundle {source}: {error}"))?;
    Ok(format!("{ASSETS}/{name}"))
}
