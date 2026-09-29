// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! What an In or Out animation does to a clip.
//!
//! A preset is a curve over its own window: `progress` 0 at the far end of
//! the motion - the clip not yet arrived, or gone - and 1 at rest. An In
//! plays 0 to 1 over the clip's first seconds; an Out plays the same curve
//! backwards, 1 to 0, over its last, so one table draws both. What comes
//! out is relative to the clip - a factor on scale and opacity, an addition
//! to position and rotation - like the keys in `animate`, so the same
//! preset means the same motion wherever the clip sits and however it is
//! keyed.

use std::f64::consts::PI;

/// What a preset does to a clip at one instant, relative to the clip's own.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Motion {
    /// A factor on the clip's scale; 1 is as placed.
    pub scale: f64,
    /// Added to the horizontal offset, in frame widths.
    pub offset_x: f64,
    /// Added to the vertical offset, in frame heights; positive is down.
    pub offset_y: f64,
    /// Added to the rotation, in degrees clockwise.
    pub rotation: f64,
    /// A factor on the clip's opacity; 1 is as set.
    pub opacity: f64,
}

impl Motion {
    /// No motion at all: the clip as it rests.
    pub const IDENTITY: Motion = Motion {
        scale: 1.0,
        offset_x: 0.0,
        offset_y: 0.0,
        rotation: 0.0,
        opacity: 1.0,
    };

    /// This motion and then `other`: factors multiply, additions add. How
    /// an In and an Out that meet on a short clip combine.
    pub fn then(self, other: Motion) -> Motion {
        Motion {
            scale: self.scale * other.scale,
            offset_x: self.offset_x + other.offset_x,
            offset_y: self.offset_y + other.offset_y,
            rotation: self.rotation + other.rotation,
            opacity: self.opacity * other.opacity,
        }
    }
}

/// Which end of a clip a preset is made for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
    /// Over the first seconds, arriving.
    In,
    /// Over the last seconds, leaving.
    Out,
}

/// Which shelf of the Animations tab a preset sits on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Group {
    /// Movement and opacity alone.
    Basic,
    /// A video effect ramped over the window, perhaps with movement too.
    Effects,
}

/// A video effect a preset ramps over its window: the package `effect`'s
/// parameter `param` runs from `from` at the far end to `rest` at rest, and
/// the link is gone outside the window.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct EffectRamp {
    /// The package id, e.g. "concat.gaussian-blur".
    pub effect: &'static str,
    /// Its parameter's key, e.g. "radius".
    pub param: &'static str,
    /// The value at the far end of the motion.
    pub from: f64,
    /// The value at rest.
    pub rest: f64,
}

/// One entry of the preset table.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Preset {
    /// Stable forever: project files store it.
    pub id: &'static str,
    /// The end it is made for; the tab offers it on that end only.
    pub side: Side,
    /// The shelf it sits on.
    pub group: Group,
    /// The i18n key of its name, in full.
    pub label: &'static str,
    curve: Curve,
    /// The effect it ramps, for an Effects preset.
    pub effect: Option<EffectRamp>,
}

/// The shapes of motion, each over progress 0..=1 towards rest.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Curve {
    /// Nothing moves; the effect does the work.
    #[allow(dead_code)] // the Effects presets of Task 6
    Still,
    /// Opacity alone, eased at both ends.
    Fade,
    /// Opacity alone, reaching full at the halfway point.
    #[allow(dead_code)] // the Effects presets of Task 6
    FadeFast,
    /// Scale from this factor, fading up as it goes.
    Zoom(f64),
    /// Position from this offset, fading up as it goes.
    Slide(f64, f64),
    /// A half turn and a grow from small.
    Spin,
    /// Grow from nothing, past full size, and back.
    Pop,
    /// A sideways wobble that dies away.
    Shake,
}

const fn basic(id: &'static str, side: Side, label: &'static str, curve: Curve) -> Preset {
    Preset {
        id,
        side,
        group: Group::Basic,
        label,
        curve,
        effect: None,
    }
}

/// Every preset, In and Out, in the order the tab shows them.
pub const PRESETS: &[Preset] = &[
    basic("fade-in", Side::In, "animations.preset.fadeIn", Curve::Fade),
    basic(
        "zoom-in",
        Side::In,
        "animations.preset.zoomIn",
        Curve::Zoom(0.6),
    ),
    basic(
        "zoom-from-big",
        Side::In,
        "animations.preset.zoomFromBig",
        Curve::Zoom(1.6),
    ),
    basic(
        "slide-from-left",
        Side::In,
        "animations.preset.slideFromLeft",
        Curve::Slide(-0.3, 0.0),
    ),
    basic(
        "slide-from-right",
        Side::In,
        "animations.preset.slideFromRight",
        Curve::Slide(0.3, 0.0),
    ),
    basic(
        "slide-from-top",
        Side::In,
        "animations.preset.slideFromTop",
        Curve::Slide(0.0, -0.3),
    ),
    basic(
        "slide-from-bottom",
        Side::In,
        "animations.preset.slideFromBottom",
        Curve::Slide(0.0, 0.3),
    ),
    basic(
        "rise",
        Side::In,
        "animations.preset.rise",
        Curve::Slide(0.0, 0.08),
    ),
    basic("spin-in", Side::In, "animations.preset.spinIn", Curve::Spin),
    basic("pop", Side::In, "animations.preset.pop", Curve::Pop),
    basic(
        "shake-in",
        Side::In,
        "animations.preset.shakeIn",
        Curve::Shake,
    ),
    basic(
        "fade-out",
        Side::Out,
        "animations.preset.fadeOut",
        Curve::Fade,
    ),
    basic(
        "zoom-out",
        Side::Out,
        "animations.preset.zoomOut",
        Curve::Zoom(0.6),
    ),
    basic(
        "zoom-to-big",
        Side::Out,
        "animations.preset.zoomToBig",
        Curve::Zoom(1.6),
    ),
    basic(
        "slide-to-left",
        Side::Out,
        "animations.preset.slideToLeft",
        Curve::Slide(-0.3, 0.0),
    ),
    basic(
        "slide-to-right",
        Side::Out,
        "animations.preset.slideToRight",
        Curve::Slide(0.3, 0.0),
    ),
    basic(
        "slide-to-top",
        Side::Out,
        "animations.preset.slideToTop",
        Curve::Slide(0.0, -0.3),
    ),
    basic(
        "slide-to-bottom",
        Side::Out,
        "animations.preset.slideToBottom",
        Curve::Slide(0.0, 0.3),
    ),
    basic(
        "sink",
        Side::Out,
        "animations.preset.sink",
        Curve::Slide(0.0, 0.08),
    ),
    basic(
        "spin-out",
        Side::Out,
        "animations.preset.spinOut",
        Curve::Spin,
    ),
    basic("pop-out", Side::Out, "animations.preset.popOut", Curve::Pop),
    basic(
        "shake-out",
        Side::Out,
        "animations.preset.shakeOut",
        Curve::Shake,
    ),
];

/// The preset of this id, if this build knows it.
pub fn preset(id: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|preset| preset.id == id)
}

/// The preset `id` at `progress` through it, 0 the far end and 1 at rest,
/// clamped. An unknown id stands still.
pub fn motion_at(id: &str, progress: f64) -> Motion {
    preset(id).map_or(Motion::IDENTITY, |preset| {
        curve_at(preset.curve, progress.clamp(0.0, 1.0))
    })
}

/// A preset placed on one end of a clip: which, and over how many seconds.
#[derive(Clone, PartialEq, Debug)]
pub struct Played {
    /// The preset's id; an unknown one plays nothing.
    pub id: String,
    /// Seconds of the clip it covers. Zero or less plays nothing.
    pub seconds: f64,
}

/// Arrives fast and settles: the shape most movement wants.
fn ease_out(p: f64) -> f64 {
    1.0 - (1.0 - p).powi(3)
}

/// Gentle at both ends, for a fade.
fn smooth(p: f64) -> f64 {
    p * p * (3.0 - 2.0 * p)
}

/// Past the target and back, about a tenth over: a pop.
fn back(p: f64) -> f64 {
    let c1 = 1.70158;
    let c3 = c1 + 1.0;
    1.0 + c3 * (p - 1.0).powi(3) + c1 * (p - 1.0).powi(2)
}

fn curve_at(curve: Curve, p: f64) -> Motion {
    let e = ease_out(p);
    let rest = Motion::IDENTITY;
    match curve {
        Curve::Still => rest,
        Curve::Fade => Motion {
            opacity: smooth(p),
            ..rest
        },
        Curve::FadeFast => Motion {
            opacity: (2.0 * p).min(1.0),
            ..rest
        },
        Curve::Zoom(from) => Motion {
            scale: from + (1.0 - from) * e,
            opacity: e,
            ..rest
        },
        Curve::Slide(dx, dy) => Motion {
            offset_x: dx * (1.0 - e),
            offset_y: dy * (1.0 - e),
            opacity: e,
            ..rest
        },
        Curve::Spin => Motion {
            rotation: -180.0 * (1.0 - e),
            scale: 0.3 + 0.7 * e,
            opacity: e,
            ..rest
        },
        Curve::Pop => Motion {
            scale: back(p).max(0.0),
            opacity: (4.0 * p).min(1.0),
            ..rest
        },
        Curve::Shake => Motion {
            offset_x: 0.03 * (1.0 - p) * (8.0 * PI * p).sin(),
            opacity: (4.0 * p).min(1.0),
            ..rest
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn every_preset_is_at_rest_at_the_end_of_its_window() {
        for preset in PRESETS {
            let rest = motion_at(preset.id, 1.0);
            assert!(near(rest.scale, 1.0), "{}: {rest:?}", preset.id);
            assert!(
                near(rest.offset_x, 0.0) && near(rest.offset_y, 0.0),
                "{}",
                preset.id
            );
            assert!(
                near(rest.rotation, 0.0) && near(rest.opacity, 1.0),
                "{}",
                preset.id
            );
        }
    }

    #[test]
    fn the_far_end_is_what_the_table_says() {
        let zoom = motion_at("zoom-in", 0.0);
        assert!(near(zoom.scale, 0.6) && near(zoom.opacity, 0.0));
        assert!(near(motion_at("zoom-from-big", 0.0).scale, 1.6));
        assert!(near(motion_at("slide-from-left", 0.0).offset_x, -0.3));
        assert!(near(motion_at("slide-from-right", 0.0).offset_x, 0.3));
        assert!(near(motion_at("slide-from-top", 0.0).offset_y, -0.3));
        assert!(near(motion_at("slide-from-bottom", 0.0).offset_y, 0.3));
        assert!(near(motion_at("rise", 0.0).offset_y, 0.08));
        assert!(near(motion_at("spin-in", 0.0).rotation, -180.0));
        assert!(near(motion_at("fade-in", 0.0).opacity, 0.0));
        assert!(near(motion_at("fade-in", 0.5).opacity, 0.5));
    }

    #[test]
    fn an_out_is_its_in_played_backwards() {
        for (way_in, way_out) in [
            ("fade-in", "fade-out"),
            ("zoom-in", "zoom-out"),
            ("zoom-from-big", "zoom-to-big"),
            ("slide-from-left", "slide-to-left"),
            ("slide-from-right", "slide-to-right"),
            ("slide-from-top", "slide-to-top"),
            ("slide-from-bottom", "slide-to-bottom"),
            ("rise", "sink"),
            ("spin-in", "spin-out"),
            ("pop", "pop-out"),
            ("shake-in", "shake-out"),
        ] {
            assert_eq!(preset(way_in).expect("in").side, Side::In);
            assert_eq!(preset(way_out).expect("out").side, Side::Out);
            for step in 0..=10 {
                let p = f64::from(step) / 10.0;
                assert_eq!(
                    motion_at(way_in, p),
                    motion_at(way_out, p),
                    "{way_out} at {p}"
                );
            }
        }
    }

    #[test]
    fn pop_overshoots_before_it_settles() {
        let peak = (0..=100)
            .map(|step| motion_at("pop", f64::from(step) / 100.0).scale)
            .fold(0.0, f64::max);
        assert!(peak > 1.05, "{peak}");
    }

    #[test]
    fn every_value_between_stays_in_range() {
        for preset in PRESETS {
            for step in 0..=20 {
                let m = motion_at(preset.id, f64::from(step) / 20.0);
                assert!((0.0..=1.0).contains(&m.opacity), "{}: {m:?}", preset.id);
                assert!(m.scale >= 0.0 && m.scale < 2.0, "{}: {m:?}", preset.id);
            }
        }
    }

    #[test]
    fn an_unknown_id_stands_still() {
        assert_eq!(motion_at("from-a-newer-build", 0.0), Motion::IDENTITY);
        assert!(preset("from-a-newer-build").is_none());
    }

    #[test]
    fn ids_are_unique_and_every_label_is_an_i18n_key() {
        for (index, preset) in PRESETS.iter().enumerate() {
            assert!(
                PRESETS[index + 1..]
                    .iter()
                    .all(|other| other.id != preset.id),
                "{} twice",
                preset.id
            );
            assert!(
                preset.label.starts_with("animations.preset."),
                "{}",
                preset.id
            );
        }
    }
}
