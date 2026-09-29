// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! The Remove Silences sheet: the tray's tool for taking the pauses out of
//! the selected clips.
//!
//! What it works on is fixed when it opens: the selected clips with sound,
//! a video and its detached sound counted once. Every setting change reads
//! the pauses again off the waveforms the lanes already hold
//! ([`concat_media::silence::find_silences`]); Apply sends one batch, one
//! undo step.

use std::collections::HashSet;
use std::sync::Arc;

use concat_media::Pyramid;
use concat_media::silence::{SilenceSettings, find_silences};
use concat_project::Command;

/// Why a selected clip is left out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skip {
    /// Nothing to hear: a title, a still, a silent video.
    NoSound,
    /// Its speed changes over its length.
    SpeedCurve,
    /// Its waveform is still being read.
    Loading,
}

/// Where one clip sits: what mapping source time onto it needs.
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    /// The lane, as the lanes number them.
    pub row: i32,
    /// Timeline seconds.
    pub start: f64,
    /// Timeline seconds.
    pub duration: f64,
    /// Source seconds at `start`.
    pub source_start: f64,
    /// Source seconds per timeline second.
    pub speed: f64,
}

impl Placement {
    /// Source seconds as timeline seconds on this clip, clamped to it.
    fn at(&self, source: f64) -> f64 {
        (self.start + (source - self.source_start) / self.speed)
            .clamp(self.start, self.start + self.duration)
    }
}

/// One thing the sheet cuts: a clip, or a video and its detached sound.
#[derive(Clone, Debug)]
pub struct Subject {
    /// The id the command names.
    pub clip_id: String,
    /// The clip whose sound is read.
    pub heard: Placement,
    /// Every clip the cut lands on: where the shading goes.
    pub members: Vec<Placement>,
    /// The heard clip's waveform, or why there is none to read.
    pub peaks: Result<Arc<Pyramid>, Skip>,
}

/// What the settings find over every subject.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    /// One `RemoveClipRanges` per subject with something to cut.
    pub commands: Vec<Command>,
    /// `(row, start, length)` in timeline seconds: the lanes' shading.
    pub shades: Vec<(i32, f64, f64)>,
    /// How many spans go.
    pub pauses: usize,
    /// Timeline seconds that go.
    pub removed: f64,
    /// Timeline seconds of the subjects that were read.
    pub before: f64,
    /// The subjects left out, one entry each.
    pub skipped: Vec<Skip>,
    /// Some subject is quiet from end to end.
    pub whole: bool,
}

/// Reads every subject's pauses by `settings`.
pub fn plan(subjects: &[Subject], settings: &SilenceSettings) -> Plan {
    let mut plan = Plan::default();
    for subject in subjects {
        let peaks = match &subject.peaks {
            Ok(peaks) => peaks,
            Err(skip) => {
                plan.skipped.push(*skip);
                continue;
            }
        };
        let heard = &subject.heard;
        let from = heard.source_start;
        let to = heard.source_start + heard.duration * heard.speed;
        let ranges = find_silences(peaks.finest(), from, to, settings);
        let removed: f64 = ranges.iter().map(|(a, b)| (b - a) / heard.speed).sum();
        plan.before += heard.duration;
        plan.removed += removed;
        plan.pauses += ranges.len();
        if !ranges.is_empty() && removed >= heard.duration - 1e-6 {
            plan.whole = true;
        }
        for member in &subject.members {
            for (a, b) in &ranges {
                let (x, y) = (member.at(*a), member.at(*b));
                if y > x {
                    plan.shades.push((member.row, x, y - x));
                }
            }
        }
        if !ranges.is_empty() {
            plan.commands.push(Command::RemoveClipRanges {
                clip_id: subject.clip_id.clone(),
                ranges,
            });
        }
    }
    plan
}

/// A length for the summary: "45 s", "1 m 32 s", "0.4 s".
pub fn clock(seconds: f64) -> String {
    if seconds < 1.0 {
        return format!("{seconds:.1} s");
    }
    let whole = seconds.round() as i64;
    if whole < 60 {
        format!("{whole} s")
    } else {
        format!("{} m {:02} s", whole / 60, whole % 60)
    }
}

/// One id per selected group, in selection order: a video and the sounds
/// detached from it are one group, named by the video, or by the sound
/// when its video is gone. `clips` is every clip as `(id, detached_from)`.
pub fn leaders(clips: &[(String, Option<String>)], selection: &[String]) -> Vec<String> {
    let exists = |id: &str| clips.iter().any(|(other, _)| other == id);
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for id in selection {
        let Some((_, detached_from)) = clips.iter().find(|(other, _)| other == id) else {
            continue;
        };
        let leader = match detached_from {
            Some(video) if exists(video) => video.clone(),
            _ => id.clone(),
        };
        if seen.insert(leader.clone()) {
            out.push(leader);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use concat_media::Peaks;

    /// A second of speech, a second of quiet, a second of speech, at a
    /// thousand buckets a second.
    fn speech_pause_speech() -> Arc<Pyramid> {
        let amplitude = |index: usize| if (1000..2000).contains(&index) { 0.0 } else { 0.5 };
        let max: Vec<f32> = (0..3000).map(amplitude).collect();
        let min = max.iter().map(|value| -value).collect();
        Arc::new(Pyramid::of(Peaks {
            min,
            max,
            buckets_per_second: 1000.0,
        }))
    }

    fn placed(row: i32, start: f64) -> Placement {
        Placement {
            row,
            start,
            duration: 3.0,
            source_start: 0.0,
            speed: 1.0,
        }
    }

    #[test]
    fn a_subject_becomes_one_command_and_a_shade_per_member() {
        let subject = Subject {
            clip_id: "c1".to_owned(),
            heard: placed(1, 10.0),
            members: vec![placed(0, 10.0), placed(1, 10.0)],
            peaks: Ok(speech_pause_speech()),
        };
        let plan = plan(&[subject], &SilenceSettings::default());
        assert_eq!(plan.pauses, 1);
        assert_eq!(plan.commands.len(), 1);
        match &plan.commands[0] {
            Command::RemoveClipRanges { clip_id, ranges } => {
                assert_eq!(clip_id, "c1");
                assert!((ranges[0].0 - 1.105).abs() < 0.011, "{ranges:?}");
            }
            other => panic!("not a cut: {other:?}"),
        }
        assert_eq!(plan.shades.len(), 2);
        assert_eq!(plan.shades[0].0, 0);
        assert!((plan.shades[0].1 - 11.105).abs() < 0.011, "{:?}", plan.shades);
        assert!((plan.removed - 0.79).abs() < 0.02);
        assert_eq!(plan.before, 3.0);
        assert!(!plan.whole);
    }

    #[test]
    fn a_skipped_subject_is_counted_and_cut_nowhere() {
        let loading = Subject {
            clip_id: "c2".to_owned(),
            heard: placed(0, 0.0),
            members: vec![placed(0, 0.0)],
            peaks: Err(Skip::Loading),
        };
        let plan = plan(&[loading], &SilenceSettings::default());
        assert_eq!(plan.skipped, vec![Skip::Loading]);
        assert!(plan.commands.is_empty());
        assert_eq!(plan.before, 0.0);
    }

    #[test]
    fn a_quiet_clip_is_whole() {
        let quiet = Arc::new(Pyramid::of(Peaks {
            min: vec![0.0; 3000],
            max: vec![0.0; 3000],
            buckets_per_second: 1000.0,
        }));
        let subject = Subject {
            clip_id: "c3".to_owned(),
            heard: placed(0, 0.0),
            members: vec![placed(0, 0.0)],
            peaks: Ok(quiet),
        };
        assert!(plan(&[subject], &SilenceSettings::default()).whole);
    }

    #[test]
    fn a_fast_clip_removes_timeline_seconds() {
        let subject = Subject {
            clip_id: "c4".to_owned(),
            heard: Placement {
                speed: 2.0,
                duration: 1.5,
                ..placed(0, 0.0)
            },
            members: vec![],
            peaks: Ok(speech_pause_speech()),
        };
        let plan = plan(&[subject], &SilenceSettings::default());
        assert!((plan.removed - 0.395).abs() < 0.01, "{}", plan.removed);
    }

    #[test]
    fn a_video_and_its_detached_sound_are_one_subject() {
        let clips = vec![
            ("v".to_owned(), None),
            ("s".to_owned(), Some("v".to_owned())),
            ("m".to_owned(), None),
            ("orphan".to_owned(), Some("gone".to_owned())),
        ];
        let selection: Vec<String> = ["s", "v", "m", "orphan"].map(String::from).to_vec();
        assert_eq!(leaders(&clips, &selection), vec!["v", "m", "orphan"]);
    }

    #[test]
    fn lengths_read_as_a_person_says_them() {
        assert_eq!(clock(0.4), "0.4 s");
        assert_eq!(clock(45.2), "45 s");
        assert_eq!(clock(92.0), "1 m 32 s");
        assert_eq!(clock(3605.0), "60 m 05 s");
    }
}
