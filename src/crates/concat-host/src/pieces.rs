// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Piece bundles on disk.
//!
//! A piece is a folder in the app config directory's `pieces/`:
//!
//! - `piece.json` - the sealed mini-edit ([`concat_project::model::Piece`])
//!   and the bin items and fonts it uses, with paths relative to the
//!   bundle (`assets/...`).
//! - `assets/` - those files, copied in at save time.
//! - `poster.jpg` - a still of its first picture, for the shelf card.
//!
//! Saving packs the selected clips into a bundle; loading copies a bundle's
//! files into a project folder of its own under `assets/pieces/`, ready for
//! `Command::InsertPiece`.

use std::path::{Path, PathBuf};

use concat_project::Project;
use concat_project::model::{CustomFont, MediaItem, Piece};
use concat_project::pieces::{Selection, from_selection};
use serde::{Deserialize, Serialize};

use crate::bundle::{ASSETS, bundle_file};
use crate::projects;

const MANIFEST: &str = "piece.json";
const POSTER: &str = "poster.jpg";

/// One piece, as the shelf sees it.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PieceInfo {
    /// The bundle folder.
    pub path: String,
    /// What the piece is called.
    pub name: String,
    /// Its saved length in seconds.
    pub duration: f64,
    /// Whether it has a still middle to stretch.
    pub stretches: bool,
    /// Whether the bundle carries a poster.
    pub has_poster: bool,
}

/// What `piece.json` holds.
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    concat: String,
    version: u64,
    piece: Piece,
    #[serde(default)]
    media: Vec<MediaItem>,
    #[serde(default)]
    fonts: Vec<CustomFont>,
}

/// Where the library lives under the config directory.
pub fn pieces_dir(config: &Path) -> PathBuf {
    config.join("pieces")
}

/// `wanted`, or the first of "wanted 2", "wanted 3"... not already in the
/// library. What a save names a piece before the person renames it.
pub fn unique_name(config: &Path, wanted: &str) -> String {
    let taken = |name: &str| {
        pieces_dir(config)
            .join(projects::folder_name(name))
            .join(MANIFEST)
            .exists()
    };
    if !taken(wanted) {
        return wanted.to_owned();
    }
    (2..)
        .map(|n| format!("{wanted} {n}"))
        .find(|name| !taken(name))
        .expect("an unbounded range finds one")
}

/// Packs the clips `clip_ids` names on the active timeline into a new
/// bundle. Refuses a name already in the library rather than overwrite.
pub fn save(
    config: &Path,
    project: &Project,
    clip_ids: &[String],
    name: &str,
) -> Result<PieceInfo, String> {
    let Selection {
        piece,
        mut media,
        mut fonts,
    } = from_selection(project, clip_ids, name).map_err(|error| error.to_string())?;
    let root = pieces_dir(config).join(projects::folder_name(name));
    if root.join(MANIFEST).exists() {
        return Err(format!("a piece named {name:?} already exists"));
    }
    let assets = root.join(ASSETS);
    std::fs::create_dir_all(&assets)
        .map_err(|error| format!("could not create {}: {error}", assets.display()))?;
    let poster_source = media
        .iter()
        .find(|item| item.kind != concat_project::model::MediaKind::Audio)
        .map(|item| item.path.clone());
    for item in &mut media {
        item.path = bundle_file(&assets, &item.id.clone(), &item.path.clone())?;
        item.piece_media = true;
    }
    for (index, font) in fonts.iter_mut().enumerate() {
        font.path = bundle_file(&assets, &format!("font{index}"), &font.path.clone())?;
    }
    let manifest = Manifest {
        concat: "0.1.0".to_owned(),
        version: concat_project::DOCUMENT_VERSION,
        piece,
        media,
        fonts,
    };
    let encoded = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| format!("could not encode the piece: {error}"))?;
    std::fs::write(root.join(MANIFEST), encoded)
        .map_err(|error| format!("could not write the piece: {error}"))?;
    // Best effort: a piece without a poster is still a piece.
    if let Some(source) = poster_source {
        let _ = write_poster(&source, &root.join(POSTER));
    }
    read_info(&root)
}

/// A still of `source` as the bundle's poster.
fn write_poster(source: &str, to: &Path) -> Result<(), String> {
    let frame = crate::media::still_at(source, 0.0, 480)?;
    let bytes = concat_media::jpeg(&frame, 4).map_err(crate::media::describe)?;
    std::fs::write(to, bytes).map_err(|error| error.to_string())
}

fn read_manifest(root: &Path) -> Result<Manifest, String> {
    let path = root.join(MANIFEST);
    let bytes = std::fs::read(&path)
        .map_err(|error| format!("could not read {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("{} is not a piece: {error}", path.display()))
}

fn read_info(root: &Path) -> Result<PieceInfo, String> {
    let manifest = read_manifest(root)?;
    Ok(PieceInfo {
        path: root.to_string_lossy().into_owned(),
        name: manifest.piece.name,
        duration: manifest.piece.duration,
        stretches: manifest.piece.hold.is_some(),
        has_poster: root.join(POSTER).exists(),
    })
}

/// Every piece in the library, in name order. A folder that does not parse
/// is skipped rather than sinking the shelf.
pub fn list(config: &Path) -> Vec<PieceInfo> {
    let Ok(entries) = std::fs::read_dir(pieces_dir(config)) else {
        return Vec::new();
    };
    let mut pieces: Vec<PieceInfo> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| read_info(&entry.path()).ok())
        .collect();
    pieces.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
    pieces
}

/// A bundle, ready to insert into the project at `project_dir`: its files
/// copied into `assets/pieces/<bundle folder>/` there - a folder of its own,
/// so two pieces' `hand.png` never meet - and its paths made absolute.
pub fn load(bundle: &str, project_dir: &str) -> Result<Selection, String> {
    let root = PathBuf::from(bundle);
    let manifest = read_manifest(&root)?;
    let folder = root
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "piece".to_owned());
    let destination = Path::new(project_dir)
        .join(ASSETS)
        .join("pieces")
        .join(folder);
    std::fs::create_dir_all(&destination)
        .map_err(|error| format!("could not create {}: {error}", destination.display()))?;
    let bring = |relative: &str| -> Result<String, String> {
        let Some(file) = relative.strip_prefix(&format!("{ASSETS}/")) else {
            return Ok(relative.to_owned());
        };
        let target = destination.join(file);
        std::fs::copy(root.join(relative), &target)
            .map_err(|error| format!("could not copy {relative}: {error}"))?;
        Ok(target.to_string_lossy().into_owned())
    };
    let mut media = manifest.media;
    for item in &mut media {
        item.path = bring(&item.path)?;
    }
    let mut fonts = manifest.fonts;
    for font in &mut fonts {
        font.path = bring(&font.path)?;
    }
    Ok(Selection {
        piece: manifest.piece,
        media,
        fonts,
    })
}

/// Checks that `path` is a bundle inside the library - this module deletes
/// and moves folders, and a stray path here would be a catastrophe.
fn inside(config: &Path, path: &str) -> Result<PathBuf, String> {
    let target = PathBuf::from(path);
    let library = pieces_dir(config);
    let (Ok(target), Ok(library)) = (target.canonicalize(), library.canonicalize()) else {
        return Err(format!("{path} is not a piece"));
    };
    if !target.starts_with(&library) || target == library || !target.join(MANIFEST).is_file() {
        return Err(format!("{path} is not a piece"));
    }
    Ok(target)
}

/// Gives a piece a new name: its manifest and its folder both.
pub fn rename(config: &Path, path: &str, name: &str) -> Result<PieceInfo, String> {
    let from = inside(config, path)?;
    let to = pieces_dir(config).join(projects::folder_name(name));
    if to.join(MANIFEST).exists() {
        return Err(format!("a piece named {name:?} already exists"));
    }
    let mut manifest = read_manifest(&from)?;
    manifest.piece.name = name.to_owned();
    let encoded = serde_json::to_vec_pretty(&manifest)
        .map_err(|error| format!("could not encode the piece: {error}"))?;
    std::fs::write(from.join(MANIFEST), encoded)
        .map_err(|error| format!("could not write the piece: {error}"))?;
    std::fs::rename(&from, &to).map_err(|error| format!("could not rename: {error}"))?;
    read_info(&to)
}

/// Removes one piece for good.
pub fn delete(config: &Path, path: &str) -> Result<(), String> {
    let target = inside(config, path)?;
    std::fs::remove_dir_all(&target)
        .map_err(|error| format!("could not delete {}: {error}", target.display()))
}

/// The poster bytes for one bundle, or an error the caller treats as "no art".
pub fn poster(path: &str) -> Result<Vec<u8>, String> {
    std::fs::read(Path::new(path).join(POSTER))
        .map_err(|error| format!("no poster for {path}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use concat_project::Editor;
    use concat_project::commands::{Command, NewMedia};
    use concat_project::model::MediaKind;

    fn still(path: &std::path::Path) -> NewMedia {
        NewMedia {
            path: path.to_string_lossy().into_owned(),
            name: "hand.png".to_owned(),
            duration: None,
            kind: MediaKind::Image,
            width: Some(400),
            height: Some(400),
            frame_rate: None,
            frame_rate_fraction: None,
            video_codec: None,
            audio_codec: None,
            has_audio: false,
            audio_tracks: Vec::new(),
            origin: None,
            color_space: Default::default(),
        }
    }

    /// An editor holding one still clip of a real file under `dir`.
    fn one_clip(dir: &std::path::Path, file: &str, bytes: &[u8]) -> (Editor, String) {
        std::fs::create_dir_all(dir).expect("dir");
        let path = dir.join(file);
        std::fs::write(&path, bytes).expect("writes");
        let mut editor = Editor::new();
        let media = editor
            .apply(Command::AddMedia { item: still(&path) })
            .expect("adds")
            .created_id
            .expect("id");
        let clip = editor
            .apply(Command::AddClip {
                media_id: media,
                track_id: "T1".to_owned(),
                start: 2.0,
                ripple: false,
            })
            .expect("adds")
            .created_id
            .expect("id");
        (editor, clip)
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("concat-pieces-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_piece_saves_lists_loads_and_places() {
        let root = scratch("round-trip");
        let config = root.join("config");
        let (editor, clip) = one_clip(&root.join("mine"), "hand.png", b"png");
        let info = save(&config, editor.project(), &[clip], "Point").expect("saves");
        assert_eq!(info.name, "Point");
        assert_eq!(list(&config).len(), 1);
        assert!(
            save(&config, editor.project(), &[], "Other").is_err(),
            "nothing selected"
        );

        let project_dir = root.join("project");
        std::fs::create_dir_all(&project_dir).expect("dir");
        let loaded = load(&info.path, &project_dir.to_string_lossy()).expect("loads");
        let file = std::path::Path::new(&loaded.media[0].path);
        assert!(file.is_absolute() && file.is_file(), "{file:?}");
        assert!(file.starts_with(project_dir.join("assets").join("pieces")));

        let mut fresh = Editor::new();
        fresh
            .apply(Command::InsertPiece {
                piece: loaded.piece,
                media: loaded.media,
                fonts: loaded.fonts,
                track_id: None,
                start: 0.0,
            })
            .expect("places");
        assert_eq!(fresh.project().pieces.len(), 1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn two_pieces_with_the_same_file_name_keep_their_own() {
        let root = scratch("same-name");
        let config = root.join("config");
        let (a, clip_a) = one_clip(&root.join("a"), "hand.png", b"left");
        let (b, clip_b) = one_clip(&root.join("b"), "hand.png", b"right");
        let one = save(&config, a.project(), &[clip_a], "Left").expect("saves");
        let two = save(&config, b.project(), &[clip_b], "Right").expect("saves");
        let project_dir = root.join("project");
        std::fs::create_dir_all(&project_dir).expect("dir");
        let dir = project_dir.to_string_lossy().into_owned();
        let left = load(&one.path, &dir).expect("loads");
        let right = load(&two.path, &dir).expect("loads");
        assert_ne!(left.media[0].path, right.media[0].path);
        assert_eq!(std::fs::read(&left.media[0].path).expect("reads"), b"left");
        assert_eq!(
            std::fs::read(&right.media[0].path).expect("reads"),
            b"right"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn names_are_never_overwritten_and_delete_stays_in_the_library() {
        let root = scratch("names");
        let config = root.join("config");
        let (editor, clip) = one_clip(&root.join("mine"), "hand.png", b"png");
        save(&config, editor.project(), &[clip.clone()], "Point").expect("saves");
        assert!(save(&config, editor.project(), &[clip], "Point").is_err());
        assert_eq!(unique_name(&config, "Point"), "Point 2");
        assert!(delete(&config, &root.join("mine").to_string_lossy()).is_err());
        let info = list(&config).remove(0);
        let renamed = rename(&config, &info.path, "Pointer").expect("renames");
        assert_eq!(renamed.name, "Pointer");
        delete(&config, &renamed.path).expect("deletes");
        assert!(list(&config).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
