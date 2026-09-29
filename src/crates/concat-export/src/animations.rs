// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The effect half of a clip's In and Out: a preset that ramps an effect
//! becomes one more link on the clip's chain, for the frame plan only.
//! It is never written into the project's `video_effects`, so it never
//! shows in the Effects tab, and removing the animation removes it.

use std::collections::BTreeMap;

use concat_core::motion::{self, EffectRamp};
use concat_project::model::{AppliedFilter, Clip, KeyEase, ParamKey, Span};

use crate::flatten::export_animations;

/// The links the clip's animations add to its chain, In first. Each is
/// keyed from the ramp's `from` at the far end of its window to its
/// `rest`, and spanned to the window in source seconds - `source_start`
/// and `speed` are the clip's as the engine plays them, a still's speed
/// being one. `speed` None is a clip whose source map is a curve: its
/// links go unspanned and ride their keys, which hold at rest outside
/// the window.
pub fn animation_effects(clip: &Clip, source_start: f64, speed: Option<f64>) -> Vec<AppliedFilter> {
    let (entrance, exit) = export_animations(clip);
    let length = clip.duration;
    if length <= 0.0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    if let Some(played) = entrance
        && let Some(ramp) = motion::preset(&played.id).and_then(|preset| preset.effect)
    {
        let end = played.seconds / length;
        out.push(link(
            ramp,
            [(0.0, ramp.from), (end, ramp.rest)],
            speed.map(|speed| Span {
                from: source_start,
                to: source_start + played.seconds * speed,
            }),
        ));
    }
    if let Some(played) = exit
        && let Some(ramp) = motion::preset(&played.id).and_then(|preset| preset.effect)
    {
        let begin = (length - played.seconds) / length;
        // One second past the end: the span's `to` is itself outside, and
        // the last frame must still be inside.
        out.push(link(
            ramp,
            [(begin, ramp.rest), (1.0, ramp.from)],
            speed.map(|speed| Span {
                from: source_start + (length - played.seconds) * speed,
                to: source_start + length * speed + 1.0,
            }),
        ));
    }
    out
}

fn link(ramp: EffectRamp, keys: [(f64, f64); 2], span: Option<Span>) -> AppliedFilter {
    let mut link = AppliedFilter::new(ramp.effect);
    link.params = BTreeMap::from([(ramp.param.to_owned(), ramp.rest)]);
    link.keys = BTreeMap::from([(
        ramp.param.to_owned(),
        keys.iter()
            .map(|&(at, value)| ParamKey {
                at,
                value,
                ease: KeyEase::LINEAR,
            })
            .collect(),
    )]);
    link.span = span;
    link
}

#[cfg(test)]
mod tests {
    use super::*;
    use concat_project::model::{Clip, ClipAnimation, ClipKind};

    fn clip_with(in_id: Option<&str>, out_id: Option<&str>) -> Clip {
        let mut clip = Clip::blank("c1", "t1", ClipKind::Video, "clip", 3.0, 4.0);
        clip.source_start = 10.0;
        let animation = |id: &str| ClipAnimation {
            id: id.to_owned(),
            duration: 1.0,
        };
        clip.animation_in = in_id.map(animation);
        clip.animation_out = out_id.map(animation);
        clip
    }

    #[test]
    fn an_effect_preset_adds_one_spanned_keyed_link() {
        let links = animation_effects(&clip_with(Some("blur-in"), None), 10.0, Some(1.0));
        assert_eq!(links.len(), 1);
        let link = &links[0];
        assert_eq!(link.id, "concat.gaussian-blur");
        let span = link.span.expect("spanned");
        assert_eq!((span.from, span.to), (10.0, 11.0));
        let keys = link.keys_on("radius");
        assert_eq!(keys.len(), 2);
        assert_eq!((keys[0].at, keys[0].value), (0.0, 40.0));
        assert_eq!((keys[1].at, keys[1].value), (0.25, 1.0));
    }

    #[test]
    fn an_out_ramps_up_into_the_last_frame() {
        let links = animation_effects(&clip_with(None, Some("pixelate-out")), 10.0, Some(1.0));
        let link = &links[0];
        let span = link.span.expect("spanned");
        assert_eq!((span.from, span.to), (13.0, 14.0 + 1.0));
        let keys = link.keys_on("size");
        assert_eq!((keys[0].at, keys[0].value), (0.75, 2.0));
        assert_eq!((keys[1].at, keys[1].value), (1.0, 64.0));
    }

    #[test]
    fn an_effect_window_follows_the_speed() {
        let links = animation_effects(&clip_with(Some("glitch-in"), None), 10.0, Some(2.0));
        let span = links[0].span.expect("spanned");
        assert_eq!((span.from, span.to), (10.0, 12.0));
    }

    #[test]
    fn a_curved_clip_rides_its_keys_unspanned() {
        let links = animation_effects(&clip_with(Some("glitch-in"), None), 10.0, None);
        assert!(links[0].span.is_none());
    }

    #[test]
    fn motion_presets_add_no_links() {
        assert!(
            animation_effects(
                &clip_with(Some("zoom-in"), Some("fade-out")),
                10.0,
                Some(1.0)
            )
            .is_empty()
        );
    }
}
