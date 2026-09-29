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
use std::borrow::Cow;
use std::sync::Arc;

use serde_json::Map;

use crate::model::{
    Clip, ClipKey, ClipKind, Hold, KEY_EPSILON, KeyEase, KeyProperty, MediaItem, MediaKind, Piece,
    PieceLane, Project, TextStyle, Timeline, Track, VideoSettings,
};

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

/// How often a pair of offsets keyed apart is sampled once turned.
const RESAMPLE_RATE: f64 = 30.0;

/// Where a piece clip puts its design frame in the timeline's.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Placement {
    /// The timeline's frame, in pixels.
    pub frame: (f64, f64),
    /// The piece's design frame, in pixels.
    pub design: (f64, f64),
    /// The piece clip's offset, in fractions of the timeline's frame.
    pub offset: (f64, f64),
    /// The piece clip's rotation, degrees clockwise.
    pub rotation: f64,
    /// The piece clip's scale; 1 is the design frame fitted inside.
    pub scale: f64,
}

impl Placement {
    /// The placement `clip` gives `piece` on a timeline of `video`.
    pub fn of(clip: &Clip, piece: &Piece, video: &VideoSettings) -> Placement {
        Placement {
            frame: (
                f64::from(video.width.max(1)),
                f64::from(video.height.max(1)),
            ),
            design: (
                f64::from(piece.design_width.max(1)),
                f64::from(piece.design_height.max(1)),
            ),
            offset: (clip.offset_x, clip.offset_y),
            rotation: clip.rotation,
            scale: clip.scale,
        }
    }

    /// The design frame contain-fitted into the timeline's.
    pub fn fit(&self) -> f64 {
        (self.frame.0 / self.design.0).min(self.frame.1 / self.design.1)
    }

    /// An inner offset pair, in fractions of the design frame, as the
    /// timeline's: scaled with the piece, turned about its centre, then
    /// moved to where the piece is.
    pub fn offset(&self, x: f64, y: f64) -> (f64, f64) {
        let k = self.scale * self.fit();
        let (cx, cy) = (x * self.design.0 * k, y * self.design.1 * k);
        let (sin, cos) = self.rotation.to_radians().sin_cos();
        let (rx, ry) = (cx * cos - cy * sin, cx * sin + cy * cos);
        (
            self.offset.0 + rx / self.frame.0,
            self.offset.1 + ry / self.frame.1,
        )
    }

    /// What an inner picture's scale is multiplied by so that it comes out
    /// as big, relative to the piece, as it was in the design frame. `size`
    /// is the media's pixels; None is a picture of unknown size, which the
    /// compositor fits as if it were frame-shaped.
    pub fn scale_factor(&self, size: Option<(f64, f64)>) -> f64 {
        let (w, h) = size.unwrap_or(self.design);
        let fit_design = (self.design.0 / w).min(self.design.1 / h);
        let fit_frame = (self.frame.0 / w).min(self.frame.1 / h);
        self.scale * self.fit() * fit_design / fit_frame
    }
}

/// A title's sizes, which are fractions of the frame, carried to the
/// timeline's frame at the piece's size.
fn place_text(style: &mut TextStyle, at: &Placement) {
    let k = at.scale * at.fit();
    let tall = k * at.design.1 / at.frame.1;
    let wide = k * at.design.0 / at.frame.0;
    style.font_size *= tall;
    style.stroke_width *= tall;
    style.background_radius *= tall;
    style.background_padding_x *= tall;
    style.background_padding_y *= tall;
    style.tracking *= tall;
    style.max_height *= tall;
    style.max_width *= wide;
}

/// `clip`'s offset keys carried through `at`. Exact wherever the pair can
/// be mapped key by key; resampled where two axes keyed apart get mixed
/// by a turn.
fn place_offset_keys(clip: &Clip, out: &mut Clip, at: &Placement) {
    let xs: Vec<ClipKey> = clip.keys_on(KeyProperty::OffsetX).copied().collect();
    let ys: Vec<ClipKey> = clip.keys_on(KeyProperty::OffsetY).copied().collect();
    if xs.is_empty() && ys.is_empty() {
        return;
    }
    out.keys
        .retain(|key| !matches!(key.property, KeyProperty::OffsetX | KeyProperty::OffsetY));
    let mut push = |property, when: f64, value, ease| {
        out.keys.push(ClipKey {
            property,
            at: when,
            value,
            ease,
        })
    };
    let turned = at.rotation.rem_euclid(360.0) > 1e-9;
    if !turned {
        // Unturned, each axis maps from itself alone.
        for key in &xs {
            push(
                KeyProperty::OffsetX,
                key.at,
                at.offset(key.value, clip.offset_y).0,
                key.ease,
            );
        }
        for key in &ys {
            push(
                KeyProperty::OffsetY,
                key.at,
                at.offset(clip.offset_x, key.value).1,
                key.ease,
            );
        }
    } else {
        let together = xs.len() == ys.len()
            && xs
                .iter()
                .zip(&ys)
                .all(|(x, y)| (x.at - y.at).abs() <= KEY_EPSILON && x.ease.is(y.ease));
        let instants: Vec<(f64, KeyEase)> = if xs.is_empty() || ys.is_empty() || together {
            xs.iter().chain(&ys).map(|key| (key.at, key.ease)).fold(
                Vec::new(),
                |mut seen: Vec<(f64, KeyEase)>, (when, ease)| {
                    if !seen
                        .iter()
                        .any(|(had, _)| (had - when).abs() <= KEY_EPSILON)
                    {
                        seen.push((when, ease));
                    }
                    seen
                },
            )
        } else {
            let steps = (clip.duration * RESAMPLE_RATE).ceil().max(1.0) as usize;
            (0..=steps)
                .map(|step| (step as f64 / steps as f64, KeyEase::LINEAR))
                .collect()
        };
        for (when, ease) in instants {
            let (x, y) = at.offset(
                clip.value_at(KeyProperty::OffsetX, when),
                clip.value_at(KeyProperty::OffsetY, when),
            );
            push(KeyProperty::OffsetX, when, x, ease);
            push(KeyProperty::OffsetY, when, y, ease);
        }
    }
    out.sort_keys();
}

/// One inner clip as the timeline plays it under `at`: its transform, its
/// keys and, for a title, its sizes carried through the piece's. Sound and
/// layers have no place on screen and come back as they were.
pub fn place_clip(clip: &Clip, size: Option<(f64, f64)>, at: &Placement) -> Clip {
    let mut out = clip.clone();
    if matches!(clip.kind, ClipKind::Audio | ClipKind::Layer) {
        return out;
    }
    let factor = if clip.kind == ClipKind::Text {
        if let Some(style) = &mut out.text {
            place_text(style, at);
        }
        1.0
    } else {
        at.scale_factor(size)
    };
    out.scale = clip.scale * factor;
    (out.offset_x, out.offset_y) = at.offset(clip.offset_x, clip.offset_y);
    out.rotation = clip.rotation + at.rotation;
    for key in &mut out.keys {
        match key.property {
            KeyProperty::Scale => key.value *= factor,
            KeyProperty::Rotation => key.value += at.rotation,
            _ => {}
        }
    }
    place_offset_keys(clip, &mut out, at);
    out
}

/// A piece clip's inner lanes as they play: stretched to its length,
/// placed by its transform, retitled by its texts, and named and timed on
/// the timeline. None when the clip places no piece this project holds.
pub fn expand_clip(
    project: &Project,
    video: &VideoSettings,
    piece_clip: &Clip,
) -> Option<Vec<Vec<Clip>>> {
    let placement = piece_clip.piece.as_ref()?;
    let piece = project.piece(&placement.piece_id)?;
    let (shortest, longest) = piece.length_range(&project.media);
    let length = piece_clip.duration.clamp(shortest, longest);
    let at = Placement::of(piece_clip, piece, video);
    Some(
        piece
            .stretched(length)
            .into_iter()
            .map(|lane| {
                lane.clips
                    .into_iter()
                    .filter(|inner| inner.start < length)
                    .map(|inner| {
                        let size = project
                            .media_by_id(&inner.media_id)
                            .and_then(|item| {
                                Some((f64::from(item.width?), f64::from(item.height?)))
                            })
                            .filter(|(w, h)| *w > 0.0 && *h > 0.0);
                        let mut clip = place_clip(&inner, size, &at);
                        if let (Some(words), Some(style)) =
                            (placement.texts.get(&inner.id), clip.text.as_mut())
                        {
                            style.content = words.clone();
                        }
                        clip.id = format!("{}/{}", piece_clip.id, inner.id);
                        clip.start = piece_clip.start + inner.start;
                        // A sound that ran past a shortened piece stops with it.
                        clip.duration = clip
                            .duration
                            .min(length - inner.start)
                            .max(MIN_CLIP_DURATION);
                        clip
                    })
                    .collect()
            })
            .collect(),
    )
}

/// One timeline with its piece clips replaced by their lanes, each piece's
/// lanes inserted just above the lane it sits on.
fn expand_timeline(project: &Project, timeline: &Timeline) -> Timeline {
    let mut tracks = Vec::with_capacity(timeline.tracks.len());
    let mut clips: Vec<Arc<Clip>> = Vec::with_capacity(timeline.clips.len());
    for track in &timeline.tracks {
        let mut placed: Vec<&Clip> = timeline
            .clips
            .iter()
            .map(Arc::as_ref)
            .filter(|clip| clip.kind == ClipKind::Piece && clip.track_id == track.id)
            .collect();
        placed.sort_by(|a, b| a.start.total_cmp(&b.start));
        for piece_clip in placed {
            let Some(lanes) = expand_clip(project, &timeline.video, piece_clip) else {
                continue;
            };
            for (index, lane) in lanes.into_iter().enumerate() {
                let id = format!("{}/L{index}", piece_clip.id);
                tracks.push(Track {
                    id: id.clone(),
                    visible: track.visible,
                    muted: track.muted,
                    extra: Map::new(),
                });
                clips.extend(lane.into_iter().map(|mut clip| {
                    clip.track_id = id.clone();
                    Arc::new(clip)
                }));
            }
        }
        tracks.push(track.clone());
    }
    clips.extend(
        timeline
            .clips
            .iter()
            .filter(|clip| clip.kind != ClipKind::Piece)
            .cloned(),
    );
    Timeline {
        tracks,
        clips,
        ..timeline.clone()
    }
}

/// `project` with every piece clip on one timeline - `timeline_id`, or the
/// active one when None - replaced by the clips it plays. Borrowed as it
/// is when that timeline has no piece clips. What rendering reads; never
/// saved, never edited.
pub fn expand_pieces<'a>(project: &'a Project, timeline_id: Option<&str>) -> Cow<'a, Project> {
    let index = match timeline_id {
        Some(id) => project
            .timelines
            .iter()
            .position(|timeline| timeline.id == id),
        None => project
            .timelines
            .iter()
            .position(|timeline| timeline.id == project.active_timeline_id)
            .or((!project.timelines.is_empty()).then_some(0)),
    };
    let Some(index) = index else {
        return Cow::Borrowed(project);
    };
    let timeline = &project.timelines[index];
    if !timeline
        .clips
        .iter()
        .any(|clip| clip.kind == ClipKind::Piece)
    {
        return Cow::Borrowed(project);
    }
    let mut owned = project.clone();
    owned.timelines[index] = Arc::new(expand_timeline(project, timeline));
    Cow::Owned(owned)
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

    use crate::model::{KeyEase, TextStyle, VideoSettings};

    fn video(width: u32, height: u32) -> VideoSettings {
        VideoSettings {
            width,
            height,
            ..VideoSettings::default()
        }
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn at_rest_a_piece_changes_nothing() {
        let made = piece("p", vec![vec![hand("h", "m1")]]);
        let clip = placed("c", "T1", "p", 0.0, 2.0);
        let at = Placement::of(&clip, &made, &video(1920, 1080));
        let inner = &made.lanes[0].clips[0];
        let out = place_clip(inner, Some((400.0, 400.0)), &at);
        assert!(close(out.scale, inner.scale));
        assert!(close(out.offset_x, inner.offset_x) && close(out.offset_y, inner.offset_y));
        assert_eq!(out.keys, inner.keys);
    }

    #[test]
    fn a_wide_piece_fits_a_tall_frame() {
        let made = piece("p", vec![vec![hand("h", "m1")]]);
        let clip = placed("c", "T1", "p", 0.0, 2.0);
        let at = Placement::of(&clip, &made, &video(1080, 1920));
        assert!(close(at.fit(), 1080.0 / 1920.0));
        // A square picture 0.25 of the design width right of centre...
        let (x, y) = at.offset(0.25, 0.0);
        // ...is 0.25 * 1920 * fit = 270 px right of centre: a quarter of 1080.
        assert!(close(x, 0.25) && close(y, 0.0));
        // A 400x400 still fitted to 1080 tall at scale 1 is 1080 px; the
        // piece wants it 1080 * fit = 607.5 px; in 1080x1920 it fits to 1080
        // wide, so its scale must become 607.5 / 1080.
        assert!(close(at.scale_factor(Some((400.0, 400.0))), 607.5 / 1080.0));
    }

    #[test]
    fn turning_a_piece_turns_its_contents_about_its_centre() {
        let made = piece("p", vec![vec![hand("h", "m1")]]);
        let mut clip = placed("c", "T1", "p", 0.0, 2.0);
        clip.rotation = 90.0;
        clip.offset_x = 0.1;
        let at = Placement::of(&clip, &made, &video(1920, 1080));
        // Right of centre by a tenth of the width is 192 px; turned a quarter
        // clockwise it is 192 px below: 192 / 1080 of the height.
        let (x, y) = at.offset(0.1, 0.0);
        assert!(close(x, 0.1) && close(y, 192.0 / 1080.0), "{x} {y}");
        let inner = made.lanes[0].clips[0].clone();
        assert!(close(place_clip(&inner, None, &at).rotation, 90.0));
    }

    #[test]
    fn one_keyed_axis_turns_into_both_at_the_same_instants() {
        let made = piece("p", vec![vec![hand("h", "m1")]]);
        let mut clip = placed("c", "T1", "p", 0.0, 2.0);
        clip.rotation = 90.0;
        let at = Placement::of(&clip, &made, &video(1920, 1080));
        let out = place_clip(&made.lanes[0].clips[0], None, &at);
        let xs: Vec<f64> = out
            .keys_on(KeyProperty::OffsetX)
            .map(|key| key.at)
            .collect();
        let ys: Vec<f64> = out
            .keys_on(KeyProperty::OffsetY)
            .map(|key| key.at)
            .collect();
        assert_eq!(xs, vec![0.0, 0.3, 0.8, 1.0]);
        assert_eq!(xs, ys);
    }

    #[test]
    fn axes_keyed_apart_are_resampled_when_turned() {
        let mut inner = hand("h", "m1");
        inner.set_key(KeyProperty::OffsetY, 0.5, 0.2, KeyEase::LINEAR);
        let made = piece("p", vec![vec![inner]]);
        let mut clip = placed("c", "T1", "p", 0.0, 2.0);
        clip.rotation = 30.0;
        let at = Placement::of(&clip, &made, &video(1920, 1080));
        let out = place_clip(&made.lanes[0].clips[0], None, &at);
        // Two seconds at thirty a second, both ends included.
        assert_eq!(out.keys_on(KeyProperty::OffsetX).count(), 61);
    }

    #[test]
    fn a_title_grows_with_its_piece() {
        let made = piece("p", vec![vec![title("t", "Hi")]]);
        let mut clip = placed("c", "T1", "p", 0.0, 3.0);
        clip.scale = 2.0;
        let at = Placement::of(&clip, &made, &video(1920, 1080));
        let out = place_clip(&made.lanes[0].clips[0], None, &at);
        let before = TextStyle::default().font_size;
        assert!(close(out.text.expect("text").font_size, before * 2.0));
        assert!(close(out.scale, 1.0), "a title's size lives in its style");
    }

    #[test]
    fn expansion_replaces_the_piece_with_its_lanes_above_its_own() {
        let mut project = Project::new();
        project.media.push(image("m1", 400, 400));
        let mut made = piece("p2", vec![vec![hand("h", "m1")], vec![title("t", "Hi")]]);
        made.hold = compute_hold(&made.lanes);
        project.pieces.push(made);
        let mut clip = placed("c3", "T2", "p2", 5.0, 3.0);
        clip.piece
            .as_mut()
            .expect("placement")
            .texts
            .insert("t".to_owned(), "Hello".to_owned());
        project.active_mut().clips.push(std::sync::Arc::new(clip));
        let expanded = expand_pieces(&project, None);
        let timeline = expanded.active();
        let ids: Vec<&str> = timeline
            .tracks
            .iter()
            .map(|track| track.id.as_str())
            .collect();
        assert_eq!(ids, vec!["T1", "c3/L0", "c3/L1", "T2", "T3", "T4"]);
        assert!(timeline.clip("c3").is_none());
        let words = timeline.clip("c3/t").expect("the title");
        assert_eq!(words.text.as_ref().expect("text").content, "Hello");
        assert_eq!(words.track_id, "c3/L1");
        assert!(close(words.start, 5.0));
        let hand = timeline.clip("c3/h").expect("the hand");
        assert!(close(hand.start, 5.0));
    }

    #[test]
    fn a_hidden_lane_hides_the_piece_on_it() {
        let mut project = Project::new();
        project
            .pieces
            .push(piece("p2", vec![vec![title("t", "Hi")]]));
        project.active_mut().tracks[1].visible = false;
        project
            .active_mut()
            .clips
            .push(std::sync::Arc::new(placed("c3", "T2", "p2", 0.0, 3.0)));
        let expanded = expand_pieces(&project, None);
        assert!(!expanded.active().track("c3/L0").expect("lane").visible);
    }

    #[test]
    fn a_missing_piece_expands_to_nothing_and_no_pieces_borrows() {
        let mut project = Project::new();
        assert!(matches!(
            expand_pieces(&project, None),
            std::borrow::Cow::Borrowed(_)
        ));
        project
            .active_mut()
            .clips
            .push(std::sync::Arc::new(placed("c3", "T1", "gone", 0.0, 3.0)));
        let expanded = expand_pieces(&project, None);
        assert!(expanded.active().clips.is_empty());
        assert_eq!(expanded.active().tracks.len(), 4);
    }
}
