// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Pieces: sealed mini-edits placed as one clip.
//!
//! A [`Piece`] is stored once in [`Project::pieces`]; every placement is a
//! `ClipKind::Piece` clip that names it and carries only what a person may
//! change - the clip's own scale, offset and rotation, its length, and the
//! words of each title inside. Rendering never sees a piece: it sees what
//! [`expand_pieces`] turns one into.

use crate::model::{ClipKind, Project};

/// `project` as it should be written: pieces no clip on any timeline places
/// are left out, and so is media that came in with a piece and that no
/// remaining piece or clip uses. The editor's own copy keeps both, so an
/// undo that brings a placement back finds its piece.
pub fn in_use(project: &Project) -> Project {
    let placed: std::collections::HashSet<&str> = project
        .timelines
        .iter()
        .flat_map(|timeline| timeline.clips.iter())
        .filter(|clip| clip.kind == ClipKind::Piece)
        .filter_map(|clip| {
            clip.piece
                .as_ref()
                .map(|placement| placement.piece_id.as_str())
        })
        .collect();
    let mut out = project.clone();
    out.pieces
        .retain(|piece| placed.contains(piece.id.as_str()));
    let used: std::collections::HashSet<String> = out
        .pieces
        .iter()
        .flat_map(|piece| piece.lanes.iter().flat_map(|lane| lane.clips.iter()))
        .chain(
            out.timelines
                .iter()
                .flat_map(|timeline| timeline.clips.iter().map(|clip| clip.as_ref())),
        )
        .map(|clip| clip.media_id.clone())
        .collect();
    out.media
        .retain(|item| !item.piece_media || used.contains(&item.id));
    out
}

/// Shared by the tests here and in `commands`; not every test uses all.
#[cfg(test)]
#[allow(dead_code)]
pub(crate) mod fixtures {
    use std::collections::BTreeMap;

    use crate::model::{
        Clip, ClipAnimation, ClipKey, ClipKind, Hold, KeyEase, KeyProperty, MediaItem, MediaKind,
        Piece, PieceLane, PiecePlacement, TextStyle,
    };

    /// A title, 0-3 s, sliding in and out over half a second each.
    pub fn title(id: &str, words: &str) -> Clip {
        let mut clip = Clip::blank(id, "", ClipKind::Text, words, 0.0, 3.0);
        clip.text = Some(TextStyle {
            content: words.to_owned(),
            ..TextStyle::default()
        });
        clip.animation_in = Some(ClipAnimation {
            id: "slide-left".to_owned(),
            duration: 0.5,
        });
        clip.animation_out = Some(ClipAnimation {
            id: "fade-out".to_owned(),
            duration: 0.5,
        });
        clip
    }

    /// A still of `media_id`, 0-2 s, keyed to swoop in over its first 0.6 s
    /// and out over its last 0.4 s.
    pub fn hand(id: &str, media_id: &str) -> Clip {
        let mut clip = Clip::blank(id, "", ClipKind::Image, "hand", 0.0, 2.0);
        clip.media_id = media_id.to_owned();
        for (at, value) in [(0.0, 0.6), (0.3, 0.0), (0.8, 0.0), (1.0, 0.6)] {
            clip.keys.push(ClipKey {
                property: KeyProperty::OffsetX,
                at,
                value,
                ease: KeyEase::LINEAR,
            });
        }
        clip
    }

    /// A sound of `media_id` from `start` for `duration`.
    pub fn whoosh(id: &str, media_id: &str, start: f64, duration: f64) -> Clip {
        let mut clip = Clip::blank(id, "", ClipKind::Audio, "whoosh", start, duration);
        clip.media_id = media_id.to_owned();
        clip
    }

    pub fn image(id: &str, width: u32, height: u32) -> MediaItem {
        MediaItem {
            id: id.to_owned(),
            path: format!("/pieces/{id}.png"),
            name: id.to_owned(),
            kind: MediaKind::Image,
            width: Some(width),
            height: Some(height),
            piece_media: true,
            ..MediaItem::default()
        }
    }

    pub fn piece(id: &str, lanes: Vec<Vec<Clip>>) -> Piece {
        let lanes: Vec<PieceLane> = lanes
            .into_iter()
            .map(|clips| PieceLane {
                clips,
                ..PieceLane::default()
            })
            .collect();
        let duration = lanes
            .iter()
            .flat_map(|lane| lane.clips.iter())
            .map(|clip| clip.start + clip.duration)
            .fold(0.0, f64::max);
        Piece {
            id: id.to_owned(),
            name: id.to_owned(),
            design_width: 1920,
            design_height: 1080,
            duration,
            hold: None,
            lanes,
            ..Piece::default()
        }
    }

    pub fn with_hold(mut piece: Piece, from: f64, to: f64) -> Piece {
        piece.hold = Some(Hold { from, to });
        piece
    }

    /// A placement of `piece_id` on `track_id` from `start` for `duration`.
    pub fn placed(id: &str, track_id: &str, piece_id: &str, start: f64, duration: f64) -> Clip {
        let mut clip = Clip::blank(id, track_id, ClipKind::Piece, piece_id, start, duration);
        clip.piece = Some(PiecePlacement {
            piece_id: piece_id.to_owned(),
            texts: BTreeMap::new(),
        });
        clip
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::fixtures::*;
    use super::*;
    use crate::doc::{DocumentSettings, from_document, to_document};

    fn settings() -> DocumentSettings {
        DocumentSettings {
            name: "P".to_owned(),
            width: 1920,
            height: 1080,
            rate_num: 30,
            rate_den: 1,
        }
    }

    fn project_with_piece() -> Project {
        let mut project = Project::new();
        project.media.push(image("m1", 400, 400));
        project.pieces.push(piece(
            "p2",
            vec![vec![hand("h", "m1")], vec![title("t", "Hi")]],
        ));
        project
            .active_mut()
            .clips
            .push(Arc::new(placed("c3", "T1", "p2", 1.0, 3.0)));
        project
    }

    #[test]
    fn a_piece_round_trips_through_the_document() {
        let project = project_with_piece();
        let document = to_document(&settings(), &project);
        assert_eq!(document["pieces"][0]["id"], "p2");
        assert_eq!(document["media"][0]["pieceMedia"], true);
        let back = from_document(&document).expect("reads");
        assert_eq!(back.pieces, project.pieces);
        let clip = back.active().clip("c3").expect("the placement survives");
        assert_eq!(clip.kind, ClipKind::Piece);
        assert_eq!(clip.piece.as_ref().expect("placement").piece_id, "p2");
        assert!(clip.media_id.is_empty());
    }

    #[test]
    fn a_project_without_pieces_writes_no_piece_fields() {
        let document = to_document(&settings(), &Project::new());
        assert!(document.get("pieces").is_none());
        let text = serde_json::to_string(&document).expect("encodes");
        assert!(!text.contains("piece"), "{text}");
    }

    #[test]
    fn unused_pieces_leave_the_document_but_placed_ones_stay() {
        let mut project = project_with_piece();
        project
            .pieces
            .push(piece("p9", vec![vec![hand("h", "m1")]]));
        let written = in_use(&project);
        assert_eq!(written.pieces.len(), 1, "p9 is placed nowhere");
        assert_eq!(written.media.len(), 1, "m1 is still used by p2");

        project.active_mut().clips.clear();
        let written = in_use(&project);
        assert!(written.pieces.is_empty());
        assert!(written.media.is_empty(), "piece media nobody uses goes too");
    }

    #[test]
    fn a_piece_inside_a_piece_is_dropped_on_read() {
        let mut project = project_with_piece();
        project.pieces[0].lanes[0]
            .clips
            .push(placed("x", "", "p2", 0.0, 1.0));
        let back = from_document(&to_document(&settings(), &project)).expect("reads");
        assert!(
            back.pieces[0].lanes[0]
                .clips
                .iter()
                .all(|clip| clip.kind != ClipKind::Piece)
        );
    }

    #[test]
    fn piece_ids_are_adopted_by_the_mint() {
        let mut project = project_with_piece();
        // Higher than any other id in the project, so only the piece can
        // have moved the counter this far.
        project.pieces[0].id = "p20".to_owned();
        let mut mint = crate::commands::IdMint::default();
        mint.adopt_project(&project);
        assert_eq!(mint.next("c"), "c21");
    }
}
