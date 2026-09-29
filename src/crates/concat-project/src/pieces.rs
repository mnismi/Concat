// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Pieces: sealed mini-edits placed as one clip.
//!
//! A [`Piece`] is stored once in [`Project::pieces`]; every placement is a
//! `ClipKind::Piece` clip that names it and carries only what a person may
//! change - the clip's own scale, offset and rotation, its length, and the
//! words of each title inside. Rendering never sees a piece: it sees what
//! [`expand_pieces`] turns one into.

use crate::model::ranges::MIN_CLIP_DURATION;
use crate::model::{Clip, ClipKind, Hold, MediaItem, MediaKind, Piece, PieceLane, Project};

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

/// Two instants closer than this are the same instant.
const EPSILON: f64 = 1e-6;

/// Whether an inner clip puts something on screen. Sound does not, so it
/// neither bounds the hold nor stretches with it.
fn shows(clip: &Clip) -> bool {
    clip.kind != ClipKind::Audio
}

/// Piece seconds of everything on `clip` that changes over time: its keys,
/// its effects' keys, and the ends of its effects' spans.
fn marks(clip: &Clip) -> Vec<f64> {
    let at = |fraction: f64| clip.start + fraction * clip.duration;
    let mut out: Vec<f64> = clip.keys.iter().map(|key| at(key.at)).collect();
    let speed = clip.speed.max(1e-6);
    for effect in &clip.video_effects {
        out.extend(effect.keys.values().flatten().map(|key| at(key.at)));
        if let Some(span) = effect.span {
            for source in [span.from, span.to] {
                out.push(clip.start + (source - clip.source_start) / speed);
            }
        }
    }
    out
}

/// The still middle of a piece's lanes: from where the last entrance has
/// finished to where the first exit begins, over everything on screen.
/// None when there is no such stretch, which makes the piece fixed length.
///
/// Every picture's entrance ends no earlier than it starts and its exit
/// begins no later than it ends, so every picture spans the hold: none
/// starts, ends or changes inside it, which is what makes stretching it
/// unambiguous.
pub fn compute_hold(lanes: &[PieceLane]) -> Option<Hold> {
    let mut from = f64::NEG_INFINITY;
    let mut to = f64::INFINITY;
    let mut any = false;
    for clip in lanes
        .iter()
        .flat_map(|lane| lane.clips.iter())
        .filter(|clip| shows(clip))
    {
        any = true;
        let end = clip.start + clip.duration;
        let middle = clip.start + clip.duration / 2.0;
        let mut entry = clip.start;
        let mut exit = end;
        if let Some(animation) = &clip.animation_in {
            entry = entry.max(clip.start + animation.duration);
        }
        if let Some(animation) = &clip.animation_out {
            exit = exit.min(end - animation.duration);
        }
        for mark in marks(clip) {
            if mark <= middle {
                entry = entry.max(mark);
            } else {
                exit = exit.min(mark);
            }
        }
        if clip.speed_curve.is_some() {
            // A curve's map to the source is not a line to stretch along.
            return None;
        }
        from = from.max(entry);
        to = to.min(exit);
    }
    (any && to - from >= MIN_CLIP_DURATION).then_some(Hold { from, to })
}

impl Piece {
    /// The shortest and longest this piece can be placed at. The hold
    /// squeezes to nothing at the short end; at the long end, a video
    /// inside can only grow by the footage it has left, and anything else
    /// grows without limit. A fixed-length piece has one length.
    pub fn length_range(&self, media: &[MediaItem]) -> (f64, f64) {
        let Some(hold) = self.hold else {
            return (self.duration, self.duration);
        };
        let shortest = (self.duration - (hold.to - hold.from)).max(MIN_CLIP_DURATION);
        let mut longest = f64::INFINITY;
        for clip in self.lanes.iter().flat_map(|lane| lane.clips.iter()) {
            if clip.kind != ClipKind::Video {
                continue;
            }
            let Some(total) = media
                .iter()
                .find(|item| item.id == clip.media_id)
                .filter(|item| item.kind == MediaKind::Video)
                .and_then(|item| item.duration)
            else {
                continue;
            };
            let room = (total - clip.source_start) / clip.speed.max(1e-6) - clip.duration;
            longest = longest.min(self.duration + room.max(0.0));
        }
        (shortest, longest.max(shortest))
    }

    /// The lanes as they play at `length`: pictures grow or shrink through
    /// the hold with their marks either side kept at their real times, and
    /// sound after the hold moves with it. `length` is taken as given; the
    /// caller clamps it into [`Piece::length_range`].
    pub fn stretched(&self, length: f64) -> Vec<PieceLane> {
        let delta = length - self.duration;
        let Some(hold) = self.hold else {
            return self.lanes.clone();
        };
        if delta.abs() < EPSILON {
            return self.lanes.clone();
        }
        let shift = move |t: f64| if t >= hold.to - EPSILON { t + delta } else { t };
        self.lanes
            .iter()
            .map(|lane| PieceLane {
                clips: lane
                    .clips
                    .iter()
                    .map(|clip| {
                        let mut clip = clip.clone();
                        if !shows(&clip) {
                            if clip.start >= hold.to - EPSILON {
                                clip.start += delta;
                            }
                            return clip;
                        }
                        let (start, old) = (clip.start, clip.duration);
                        let new = (old + delta).max(MIN_CLIP_DURATION);
                        let remap =
                            |at: f64| ((shift(start + at * old) - start) / new).clamp(0.0, 1.0);
                        for key in &mut clip.keys {
                            key.at = remap(key.at);
                        }
                        let (source_start, speed) = (clip.source_start, clip.speed.max(1e-6));
                        for effect in &mut clip.video_effects {
                            for key in effect.keys.values_mut().flatten() {
                                key.at = remap(key.at);
                            }
                            if let Some(span) = &mut effect.span {
                                let moved = |source: f64| {
                                    let at = shift(start + (source - source_start) / speed);
                                    source_start + (at - start) * speed
                                };
                                span.from = moved(span.from);
                                span.to = moved(span.to);
                            }
                        }
                        clip.duration = new;
                        clip
                    })
                    .collect(),
                extra: lane.extra.clone(),
            })
            .collect()
    }
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

    use crate::model::{Hold, KeyProperty};

    #[test]
    fn a_title_holds_between_its_in_and_its_out() {
        let lanes = piece("p", vec![vec![title("t", "Hi")]]).lanes;
        assert_eq!(compute_hold(&lanes), Some(Hold { from: 0.5, to: 2.5 }));
    }

    #[test]
    fn keys_bound_the_hold_and_sound_does_not() {
        let lanes = piece(
            "p",
            vec![vec![hand("h", "m1")], vec![whoosh("w", "m2", 0.0, 1.9)]],
        )
        .lanes;
        // The hand's keys sit at 0.6 s (0.3 of 2 s) and 1.6 s (0.8 of 2 s).
        let hold = compute_hold(&lanes).expect("a still middle");
        assert!((hold.from - 0.6).abs() < 1e-9 && (hold.to - 1.6).abs() < 1e-9);
    }

    #[test]
    fn clips_in_sequence_leave_no_hold() {
        let mut second = title("t2", "Bye");
        second.start = 3.0;
        let lanes = piece("p", vec![vec![title("t1", "Hi"), second]]).lanes;
        assert_eq!(compute_hold(&lanes), None);
    }

    #[test]
    fn motion_all_the_way_through_leaves_no_hold() {
        let mut clip = title("t", "Hi");
        clip.animation_in.as_mut().expect("in").duration = 1.5;
        clip.animation_out.as_mut().expect("out").duration = 1.5;
        assert_eq!(compute_hold(&piece("p", vec![vec![clip]]).lanes), None);
    }

    #[test]
    fn stretch_keeps_the_motion_at_its_speed() {
        let made = with_hold(
            piece(
                "p",
                vec![
                    vec![hand("h", "m1")],
                    vec![whoosh("w1", "m2", 0.1, 0.4), whoosh("w2", "m2", 1.7, 0.3)],
                ],
            ),
            0.6,
            1.6,
        );
        let lanes = made.stretched(4.0); // two seconds longer
        let hand = &lanes[0].clips[0];
        assert!((hand.duration - 4.0).abs() < 1e-9);
        let times: Vec<f64> = hand
            .keys_on(KeyProperty::OffsetX)
            .map(|key| key.at * hand.duration)
            .collect();
        for (got, want) in times.iter().zip([0.0, 0.6, 3.6, 4.0]) {
            assert!((got - want).abs() < 1e-9, "{times:?}");
        }
        let sounds = &lanes[1].clips;
        assert!(
            (sounds[0].start - 0.1).abs() < 1e-9,
            "before the hold stays"
        );
        assert!(
            (sounds[0].duration - 0.4).abs() < 1e-9,
            "sound never stretches"
        );
        assert!((sounds[1].start - 3.7).abs() < 1e-9, "after the hold moves");
    }

    #[test]
    fn a_piece_shortens_down_to_its_hold_and_no_further() {
        let made = with_hold(piece("p", vec![vec![title("t", "Hi")]]), 0.5, 2.5);
        assert_eq!(made.length_range(&[]), (1.0, f64::INFINITY));
        let lanes = made.stretched(1.0);
        assert!((lanes[0].clips[0].duration - 1.0).abs() < 1e-9);
    }

    #[test]
    fn a_video_can_only_grow_by_the_footage_it_has_left() {
        let mut clip = hand("v", "m1");
        clip.kind = crate::model::ClipKind::Video;
        clip.source_start = 1.0;
        let mut media = image("m1", 1920, 1080);
        media.kind = crate::model::MediaKind::Video;
        media.duration = Some(5.0);
        let made = with_hold(piece("p", vec![vec![clip]]), 0.6, 1.6);
        // 4 s of source after the in-point, 2 s used: room for 2 more.
        let (_, longest) = made.length_range(&[media]);
        assert!((longest - 4.0).abs() < 1e-9);
    }

    #[test]
    fn a_fixed_piece_does_not_stretch() {
        let made = piece("p", vec![vec![title("t", "Hi")]]);
        assert_eq!(made.length_range(&[]), (3.0, 3.0));
        assert_eq!(made.stretched(5.0), made.lanes);
    }
}
