// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The Animations tab's arithmetic: which presets each end offers, how
//! long an animation may be, and which stretch of the timeline the
//! preview plays after a pick. No window and no project: the studio asks,
//! and paints what comes back.
#![allow(dead_code)] // wired in Task 5

use concat_core::motion::{self, Group, Preset, Side};
use concat_project::commands::AnimationSlot;
use concat_project::model::{Clip, ClipAnimation, MIN_ANIMATION};

/// The length a new animation starts at, when the clip has room for it.
pub const DEFAULT_LENGTH: f64 = 0.5;

/// The preset table's side for a slot.
pub fn side_of(slot: AnimationSlot) -> Side {
    match slot {
        AnimationSlot::In => Side::In,
        AnimationSlot::Out => Side::Out,
    }
}

/// The presets the grid offers on one end, in the table's order with the
/// Basic shelf first.
pub fn offered(slot: AnimationSlot) -> Vec<&'static Preset> {
    let side = side_of(slot);
    let mut out: Vec<&'static Preset> = motion::PRESETS
        .iter()
        .filter(|preset| preset.side == side)
        .collect();
    out.sort_by_key(|preset| preset.group == Group::Effects);
    out
}

/// What the clip has on that end.
pub fn current(clip: &Clip, slot: AnimationSlot) -> Option<&ClipAnimation> {
    match slot {
        AnimationSlot::In => clip.animation_in.as_ref(),
        AnimationSlot::Out => clip.animation_out.as_ref(),
    }
}

/// The room one end has: the clip's length less what the other end takes.
pub fn room(clip: &Clip, slot: AnimationSlot) -> f64 {
    let other = match slot {
        AnimationSlot::In => AnimationSlot::Out,
        AnimationSlot::Out => AnimationSlot::In,
    };
    (clip.duration - current(clip, other).map_or(0.0, |animation| animation.duration)).max(0.0)
}

/// The length slider's range for one end: from the shortest a command
/// keeps to the room the other end leaves.
pub fn length_range(clip: &Clip, slot: AnimationSlot) -> (f64, f64) {
    let room = room(clip, slot);
    (MIN_ANIMATION.min(room), room)
}

/// The length a preset picked on this end starts at. An end that already
/// has one keeps its length, so trying presets one after another keeps the
/// timing the person set; an empty end starts at [`DEFAULT_LENGTH`], or at
/// half a clip too short for it, never past the room.
pub fn starting_length(clip: &Clip, slot: AnimationSlot) -> f64 {
    if let Some(animation) = current(clip, slot) {
        return animation.duration;
    }
    DEFAULT_LENGTH
        .min(clip.duration / 2.0)
        .min(room(clip, slot))
}

/// The stretch of the timeline the preview plays, in seconds: the clip's
/// first `seconds` for an In, its last for an Out.
pub fn preview_window(clip: &Clip, slot: AnimationSlot, seconds: f64) -> (f64, f64) {
    let end = clip.start + clip.duration;
    match slot {
        AnimationSlot::In => (clip.start, clip.start + seconds),
        AnimationSlot::Out => (end - seconds, end),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use concat_project::model::{ClipAnimation, ClipKind};

    fn clip(duration: f64) -> Clip {
        Clip::blank("c1", "t1", ClipKind::Video, "clip", 4.0, duration)
    }

    fn with(mut clip: Clip, slot: AnimationSlot, duration: f64) -> Clip {
        let animation = Some(ClipAnimation {
            id: "fade-in".to_owned(),
            duration,
        });
        match slot {
            AnimationSlot::In => clip.animation_in = animation,
            AnimationSlot::Out => clip.animation_out = animation,
        }
        clip
    }

    #[test]
    fn each_end_offers_its_own_presets_basic_first() {
        let ins = offered(AnimationSlot::In);
        let outs = offered(AnimationSlot::Out);
        assert!(!ins.is_empty() && !outs.is_empty());
        assert!(ins.iter().all(|preset| preset.side == Side::In));
        assert!(outs.iter().all(|preset| preset.side == Side::Out));
        let first_effect = ins
            .iter()
            .position(|preset| preset.group == Group::Effects)
            .unwrap_or(ins.len());
        assert!(
            ins[first_effect..]
                .iter()
                .all(|preset| preset.group == Group::Effects)
        );
    }

    #[test]
    fn an_end_has_the_room_the_other_leaves() {
        let clip = with(clip(5.0), AnimationSlot::Out, 2.0);
        assert_eq!(room(&clip, AnimationSlot::In), 3.0);
        assert_eq!(room(&clip, AnimationSlot::Out), 5.0);
        assert_eq!(length_range(&clip, AnimationSlot::In), (MIN_ANIMATION, 3.0));
    }

    #[test]
    fn a_new_animation_starts_at_half_a_second_or_half_a_short_clip() {
        assert_eq!(
            starting_length(&clip(5.0), AnimationSlot::In),
            DEFAULT_LENGTH
        );
        assert_eq!(starting_length(&clip(0.6), AnimationSlot::In), 0.3);
        let squeezed = with(clip(5.0), AnimationSlot::Out, 4.8);
        assert!((starting_length(&squeezed, AnimationSlot::In) - 0.2).abs() < 1e-9);
    }

    #[test]
    fn an_end_that_has_one_keeps_its_length() {
        let clip = with(clip(5.0), AnimationSlot::In, 1.7);
        assert_eq!(starting_length(&clip, AnimationSlot::In), 1.7);
    }

    #[test]
    fn the_preview_plays_the_end_it_is_about() {
        let clip = clip(5.0);
        assert_eq!(preview_window(&clip, AnimationSlot::In, 1.0), (4.0, 5.0));
        assert_eq!(preview_window(&clip, AnimationSlot::Out, 1.0), (8.0, 9.0));
    }
}
