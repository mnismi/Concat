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
use concat_project::model::Clip;

use crate::i18n::{t, tf};
use crate::prefs::Preferences;
use crate::studio::Studio;
use crate::ui::SilenceSheetData;

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

/// Everything that can happen to the Remove Silences sheet.
#[derive(Clone, Debug)]
pub enum SilenceMsg {
    /// The tray's Remove Silences tool.
    Open,
    Close,
    LevelChanged(f32),
    MinPauseChanged(f32),
    PaddingChanged(f32),
    /// A waveform arrived: read the pauses again.
    Refresh,
    Apply,
}

/// The Remove Silences sheet's state.
#[derive(Default)]
pub struct SilencePane {
    pub open: bool,
    pub settings: SilenceSettings,
    /// The selection when the sheet opened: what it works on.
    selection: Vec<String>,
    plan: Plan,
}

impl SilencePane {
    /// Applies one message. The studio is the rest of the window; while
    /// this runs the studio's copy of the pane is a blank it must not read.
    pub fn update(&mut self, msg: SilenceMsg, studio: &mut Studio) {
        match msg {
            SilenceMsg::Open => {
                self.open = true;
                self.selection = studio.selection.clone();
                self.settings = remembered(&studio.prefs);
                self.refresh(studio);
            }
            SilenceMsg::Close => {
                self.close(studio);
            }
            SilenceMsg::LevelChanged(db) => {
                self.settings.level_db = db.clamp(-60.0, -20.0);
                self.refresh(studio);
            }
            SilenceMsg::MinPauseChanged(seconds) => {
                self.settings.min_pause = f64::from(seconds).clamp(0.1, 3.0);
                self.refresh(studio);
            }
            SilenceMsg::PaddingChanged(seconds) => {
                self.settings.padding = f64::from(seconds).clamp(0.0, 0.5);
                self.refresh(studio);
            }
            SilenceMsg::Refresh => {
                if self.open {
                    self.refresh(studio);
                }
            }
            SilenceMsg::Apply => {
                if !self.ready() {
                    return;
                }
                let commands = std::mem::take(&mut self.plan.commands);
                let (pauses, removed) = (self.plan.pauses, self.plan.removed);
                self.close(studio);
                studio.apply(Command::Batch { commands });
                // A leading pause takes its clip's head, id and all.
                let alive: Vec<String> = studio
                    .selection
                    .iter()
                    .filter(|id| studio.clip(id).is_some())
                    .cloned()
                    .collect();
                studio.selection = alive;
                studio.notify(&tf("silence.removedPauses", &[&pauses, &clock(removed)]), false);
            }
        }
    }

    /// The shading the lanes draw while the sheet is open.
    pub fn shades(&self) -> Vec<(i32, f64, f64)> {
        if self.open {
            self.plan.shades.clone()
        } else {
            Vec::new()
        }
    }

    /// The sheet as Slint shows it.
    pub fn data(&self) -> SilenceSheetData {
        let plan = &self.plan;
        let summary = if plan.whole {
            t("silence.wholeClipBelowLevel")
        } else if plan.before == 0.0 {
            plan.skipped.first().map(|skip| reason(*skip)).unwrap_or_default()
        } else if plan.commands.is_empty() {
            t("silence.noPausesFound")
        } else {
            tf(
                "silence.summary",
                &[
                    &plan.pauses,
                    &clock(plan.removed),
                    &clock(plan.before),
                    &clock(plan.before - plan.removed),
                ],
            )
        };
        let skipped = match plan.skipped.first() {
            Some(skip) if plan.before > 0.0 => {
                tf("silence.clipsSkipped", &[&plan.skipped.len(), &reason(*skip)])
            }
            _ => String::new(),
        };
        SilenceSheetData {
            open: self.open,
            level: self.settings.level_db,
            min_pause: self.settings.min_pause as f32,
            padding: self.settings.padding as f32,
            summary: summary.into(),
            skipped: skipped.into(),
            ready: self.ready(),
        }
    }

    fn ready(&self) -> bool {
        !self.plan.commands.is_empty() && !self.plan.whole
    }

    fn refresh(&mut self, studio: &Studio) {
        self.plan = plan(&subjects(studio, &self.selection), &self.settings);
    }

    fn close(&mut self, studio: &mut Studio) {
        if self.open {
            remember(&mut studio.prefs, &self.settings);
            studio.prefs.save(&studio.host.dirs);
        }
        self.open = false;
        self.plan = Plan::default();
    }
}

/// Why the tray's button is off for this selection, or None when it can
/// open. A waveform still loading does not keep it shut: the sheet says
/// so and fills in when it arrives.
pub fn blocked(studio: &Studio) -> Option<String> {
    if studio.selection.is_empty() {
        return Some(t("silence.selectClipWithSound"));
    }
    let subjects = subjects(studio, &studio.selection);
    if subjects
        .iter()
        .any(|subject| !matches!(subject.peaks, Err(Skip::NoSound | Skip::SpeedCurve)))
    {
        return None;
    }
    let skip = if subjects
        .iter()
        .any(|subject| matches!(subject.peaks, Err(Skip::SpeedCurve)))
    {
        Skip::SpeedCurve
    } else {
        Skip::NoSound
    };
    Some(tf("silence.cannotRemove", &[&reason(skip)]))
}

/// The selection as subjects: one per video-and-sound group.
fn subjects(studio: &Studio, selection: &[String]) -> Vec<Subject> {
    let timeline = studio.timeline();
    let links: Vec<(String, Option<String>)> = timeline
        .clips
        .iter()
        .map(|clip| (clip.id.clone(), clip.detached_from.clone()))
        .collect();
    leaders(&links, selection)
        .into_iter()
        .filter_map(|leader| {
            let group: Vec<&Clip> = timeline
                .clips
                .iter()
                .map(|clip| clip.as_ref())
                .filter(|clip| {
                    clip.id == leader || clip.detached_from.as_deref() == Some(leader.as_str())
                })
                .collect();
            let place = |clip: &Clip| Placement {
                row: studio.row_of(&clip.track_id),
                start: clip.start,
                duration: clip.duration,
                source_start: clip.source_start,
                speed: clip.speed,
            };
            // The video's own sound unless it was detached, then the first
            // detached sound, then anything in the group with sound.
            let heard = group
                .iter()
                .copied()
                .find(|clip| clip.id == leader && clip.muted != Some(true) && studio.clip_has_sound(clip))
                .or_else(|| group.iter().copied().find(|clip| clip.detached_from.is_some()))
                .or_else(|| group.iter().copied().find(|clip| studio.clip_has_sound(clip)));
            let first = *group.first()?;
            let Some(heard) = heard else {
                return Some(Subject {
                    clip_id: leader.clone(),
                    heard: place(first),
                    members: Vec::new(),
                    peaks: Err(Skip::NoSound),
                });
            };
            let peaks = if group.iter().any(|clip| clip.speed_curve.is_some()) {
                Err(Skip::SpeedCurve)
            } else {
                studio.peaks_of(heard).ok_or(Skip::Loading)
            };
            Some(Subject {
                clip_id: heard.id.clone(),
                heard: place(heard),
                members: group.iter().map(|clip| place(clip)).collect(),
                peaks,
            })
        })
        .collect()
}

/// A skip's reason, as the summary and the tray say it. Each key is
/// written out in its own `t` call so `scripts/locales.py` finds it.
fn reason(skip: Skip) -> String {
    match skip {
        Skip::NoSound => t("silence.reasonNoSound"),
        Skip::SpeedCurve => t("silence.reasonSpeedCurve"),
        Skip::Loading => t("silence.reasonLoading"),
    }
}

fn remembered(prefs: &Preferences) -> SilenceSettings {
    let defaults = SilenceSettings::default();
    SilenceSettings {
        level_db: prefs.silence_level_db.unwrap_or(defaults.level_db),
        min_pause: prefs.silence_min_pause.unwrap_or(defaults.min_pause),
        padding: prefs.silence_padding.unwrap_or(defaults.padding),
    }
}

fn remember(prefs: &mut Preferences, settings: &SilenceSettings) {
    prefs.silence_level_db = Some(settings.level_db);
    prefs.silence_min_pause = Some(settings.min_pause);
    prefs.silence_padding = Some(settings.padding);
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
