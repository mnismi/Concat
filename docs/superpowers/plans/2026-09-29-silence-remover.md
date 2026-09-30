# Remove Silences Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Select a clip, open Remove Silences from the timeline tray, tune three settings with a live summary and shaded preview, and Apply to cut the pauses out, closing the gaps, in one undo step.

**Architecture:** Detection is a pure function over the waveform peaks the timeline already caches (`concat-media`). The cut is one new document command, `RemoveClipRanges`, built from the existing split and ripple-remove (`concat-project`), which also cuts a video's detached sound. The window gets a pane and a sheet in the shape of the captions sheet, a tray button in the shape of Merge, and a shading overlay on the lanes in the shape of the drop ghost.

**Tech Stack:** Rust 1.93 (pinned), Slint 1.18, FFmpeg through `ffmpeg-the-third`, Cargo workspace at `src/`.

**Spec:** `docs/superpowers/specs/2026-09-29-silence-remover-design.md`

## Global Constraints

- **Build with Windows cargo, never Linux cargo.** From WSL, define these once per shell and use them for every build, test and git command in this plan:
  ```bash
  wcargo() { (cd /mnt/c && cmd.exe /c "set PATH=F:\\Work\\Playground\\concat-deps\\ffdl\\ffmpeg-n8.1-latest-win64-gpl-shared-8.1\\bin;F:\\Work\\Playground\\concat-deps\\sherpa\\sherpa-onnx-v1.13.7-win-x64-shared-MT-Release-lib\\lib;%PATH%&& cd /d F:\\Work\\Playground\\Concat\\src && cargo $*" 2>&1 | tr -d '\r'); }
  wgit() { (cd /mnt/c && cmd.exe /c "cd /d F:\\Work\\Playground\\Concat && git $*" 2>&1 | tr -d '\r'); }
  ```
  Paths below are relative to the repo root `F:\Work\Playground\Concat` (WSL: `/mnt/f/Work/Playground/Concat`).
- **Line endings:** the working tree is CRLF (`core.autocrlf=true`). Keep CRLF in every file you touch. After editing a file with a tool that writes LF, run `sed -i 's/\r$//; s/$/\r/' <file>`. Check with `file <file>`: it must say "with CRLF line terminators".
- **Branch:** `feat/silence-remover`. Commit with `wgit`, never WSL's git, which sees every file as changed.
- **Do not commit the spec or the plan until Task 6.** Task 6's commit lands them (user rule).
- **Defaults and ranges, verbatim from the spec:** Silence level −60 to −20 dB, default −40 dB. Minimum pause 0.1 to 3 s, default 0.5 s. Padding 0 to 0.5 s, default 0.1 s.
- **Detection constants, verbatim from the spec:** 10 ms hold (5 ms either side), loud runs shorter than 30 ms are clicks, and less than 20 ms left after padding is dropped.
- **Other tracks never move.** Only the cut clip's track, and each detached sound's track, ripple.
- **Every user-facing string** goes through `t`/`tf` (Rust) or `I18n.t` (Slint), with its English in `src/crates/concat/locales/en.json` and a translation in all 13 other locale files. The locale test fails otherwise.
- **Header:** every new `.rs` and `.slint` file starts with the two SPDX lines the other files carry:
  `// SPDX-License-Identifier: AGPL-3.0-or-later` and `// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors`.
- **Doc comments on every public item:** the workspace lints `missing_docs`.
- **Commit messages** follow the repo's style (`feat(area): sentence`) and end with the trailer `Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>`.

## Review Focus

1. **A video selected together with its own detached sound** must be cut once, not twice. This is pinned in Task 4 (`a_video_and_its_detached_sound_are_one_subject`).
2. **A trimmed, sped-up clip:** the source in-point is not 0 and speed is not 1, and the ranges must still land on the right frames. This is pinned in Task 2 (`removing_ranges_follows_a_trimmed_fast_clip`).
3. **A clip whose sound ends before its picture does:** the tail past the audio is silence and goes. This is pinned in Task 1 (`past_the_end_of_the_sound_is_silence`).
4. **Pauses closer together than two frames,** or a range reaching within a frame of the clip's edge, must not leave slivers or fail to split. This is pinned in Task 2 (`ranges_closer_than_two_frames_merge_and_edges_snap`).
5. **An hour-long recording:** the preview recomputes on every setting change and must stay fast. This is pinned in Task 1 (`an_hour_of_peaks_is_read_quickly`).

---

### Task 1: Detection

**Files:**
- Create: `src/crates/concat-media/src/silence.rs`
- Modify: `src/crates/concat-media/src/lib.rs` (the module list at lines 16–27)
- Test: in `silence.rs`, `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `crate::peaks::Peaks { min: Vec<f32>, max: Vec<f32>, buckets_per_second: f32 }` and `Peaks::len()`.
- Produces:
  ```rust
  pub struct SilenceSettings { pub level_db: f32, pub min_pause: f64, pub padding: f64 } // Default: -40.0, 0.5, 0.1
  pub fn find_silences(peaks: &Peaks, from: f64, to: f64, settings: &SilenceSettings) -> Vec<(f64, f64)>;
  ```
  The spans are source seconds, sorted and disjoint, each within `[from, to]`.

- [ ] **Step 1: Declare the module**

In `src/crates/concat-media/src/lib.rs`, after `pub mod samples;`, add:
```rust
pub mod silence;
```

- [ ] **Step 2: Write the failing tests**

Create `src/crates/concat-media/src/silence.rs` with the header, a stub, and the tests:
```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Where a recording goes quiet: the spans Remove Silences takes out.
//!
//! Read off the waveform peaks the timeline already holds (see
//! [`crate::peaks`]), so finding the pauses costs no decode and a setting
//! can change under the pointer. Pure: peaks and numbers in, spans of the
//! source out.

use crate::peaks::Peaks;

/// How a pause is told from the rest.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SilenceSettings {
    /// Quieter than this, in dBFS, is silence.
    pub level_db: f32,
    /// A quiet run shorter than this, in seconds, is a pause between words
    /// and stays.
    pub min_pause: f64,
    /// Sound kept at each side of what is taken out, in seconds, so words
    /// keep their edges.
    pub padding: f64,
}

impl Default for SilenceSettings {
    fn default() -> Self {
        SilenceSettings {
            level_db: -40.0,
            min_pause: 0.5,
            padding: 0.1,
        }
    }
}

/// The spans of `from..to` (source seconds) that are silence by
/// `settings`, sorted and disjoint.
pub fn find_silences(
    _peaks: &Peaks,
    _from: f64,
    _to: f64,
    _settings: &SilenceSettings,
) -> Vec<(f64, f64)> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Peaks at a thousand buckets a second from `(seconds, amplitude)`
    /// stretches, each bucket reaching the amplitude either way.
    fn peaks(stretches: &[(f64, f32)]) -> Peaks {
        let mut min = Vec::new();
        let mut max = Vec::new();
        for (seconds, amplitude) in stretches {
            for _ in 0..(seconds * 1000.0).round() as usize {
                min.push(-amplitude);
                max.push(*amplitude);
            }
        }
        Peaks {
            min,
            max,
            buckets_per_second: 1000.0,
        }
    }

    fn close(spans: &[(f64, f64)], expected: &[(f64, f64)]) {
        assert_eq!(spans.len(), expected.len(), "{spans:?} vs {expected:?}");
        for (got, want) in spans.iter().zip(expected) {
            assert!(
                (got.0 - want.0).abs() < 0.011 && (got.1 - want.1).abs() < 0.011,
                "{spans:?} vs {expected:?}"
            );
        }
    }

    const SPEECH: f32 = 0.5;

    #[test]
    fn a_pause_between_speech_is_found_and_padded() {
        let p = peaks(&[(1.0, SPEECH), (1.0, 0.0), (1.0, SPEECH)]);
        let spans = find_silences(&p, 0.0, 3.0, &SilenceSettings::default());
        close(&spans, &[(1.105, 1.895)]);
    }

    #[test]
    fn a_pause_shorter_than_the_minimum_stays() {
        let p = peaks(&[(1.0, SPEECH), (0.3, 0.0), (1.0, SPEECH)]);
        assert!(find_silences(&p, 0.0, 2.3, &SilenceSettings::default()).is_empty());
    }

    #[test]
    fn the_minimum_is_measured_before_the_padding() {
        // 0.6 s is past the 0.5 s minimum, though padding leaves 0.4 s.
        let p = peaks(&[(1.0, SPEECH), (0.6, 0.0), (1.0, SPEECH)]);
        let spans = find_silences(&p, 0.0, 2.6, &SilenceSettings::default());
        close(&spans, &[(1.105, 1.495)]);
    }

    #[test]
    fn a_click_does_not_break_a_pause() {
        let p = peaks(&[
            (1.0, SPEECH),
            (0.5, 0.0),
            (0.005, 0.9),
            (0.5, 0.0),
            (1.0, SPEECH),
        ]);
        let spans = find_silences(&p, 0.0, 3.005, &SilenceSettings::default());
        close(&spans, &[(1.105, 1.9)]);
    }

    #[test]
    fn leading_and_trailing_silence_go_completely() {
        let p = peaks(&[(0.8, 0.0), (1.0, SPEECH), (0.8, 0.0)]);
        let spans = find_silences(&p, 0.0, 2.6, &SilenceSettings::default());
        close(&spans, &[(0.0, 0.695), (1.905, 2.6)]);
    }

    #[test]
    fn all_silence_is_one_span_and_no_silence_is_none() {
        let quiet = peaks(&[(3.0, 0.0)]);
        close(
            &find_silences(&quiet, 0.0, 3.0, &SilenceSettings::default()),
            &[(0.0, 3.0)],
        );
        let loud = peaks(&[(3.0, SPEECH)]);
        assert!(find_silences(&loud, 0.0, 3.0, &SilenceSettings::default()).is_empty());
    }

    #[test]
    fn padding_can_swallow_a_pause() {
        let p = peaks(&[(1.0, SPEECH), (0.6, 0.0), (1.0, SPEECH)]);
        let settings = SilenceSettings {
            padding: 0.3,
            ..SilenceSettings::default()
        };
        assert!(find_silences(&p, 0.0, 2.6, &settings).is_empty());
    }

    #[test]
    fn the_level_decides_what_is_quiet() {
        // 0.005 is about -46 dBFS: quiet at -40, loud at -50.
        let p = peaks(&[(1.0, SPEECH), (1.0, 0.005), (1.0, SPEECH)]);
        assert_eq!(
            find_silences(&p, 0.0, 3.0, &SilenceSettings::default()).len(),
            1
        );
        let strict = SilenceSettings {
            level_db: -50.0,
            ..SilenceSettings::default()
        };
        assert!(find_silences(&p, 0.0, 3.0, &strict).is_empty());
    }

    #[test]
    fn only_the_asked_window_is_read() {
        let p = peaks(&[(1.0, SPEECH), (1.0, 0.0), (1.0, SPEECH)]);
        let spans = find_silences(&p, 0.5, 2.5, &SilenceSettings::default());
        close(&spans, &[(1.105, 1.895)]);
    }

    #[test]
    fn past_the_end_of_the_sound_is_silence() {
        let p = peaks(&[(1.0, SPEECH)]);
        let spans = find_silences(&p, 0.0, 2.0, &SilenceSettings::default());
        close(&spans, &[(1.105, 2.0)]);
    }

    #[test]
    fn an_empty_window_finds_nothing() {
        let p = peaks(&[(1.0, 0.0)]);
        assert!(find_silences(&p, 1.0, 1.0, &SilenceSettings::default()).is_empty());
        assert!(find_silences(&p, 2.0, 1.0, &SilenceSettings::default()).is_empty());
    }

    #[test]
    fn an_hour_of_peaks_is_read_quickly() {
        let minute: Vec<(f64, f32)> = (0..30).flat_map(|_| [(1.5, SPEECH), (0.5, 0.0)]).collect();
        let hour: Vec<(f64, f32)> = (0..60).flat_map(|_| minute.clone()).collect();
        let p = peaks(&hour);
        let started = std::time::Instant::now();
        let spans = find_silences(&p, 0.0, 3600.0, &SilenceSettings::default());
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "{:?} for an hour",
            started.elapsed()
        );
        assert!(spans.is_empty(), "0.5 s pauses less the hold stay under the minimum");
    }
}
```

- [ ] **Step 3: Run the tests to see them fail**

Run: `wcargo test -p concat-media silence`
Expected: FAIL. The tests that expect spans fail (`a_pause_between_speech_is_found_and_padded` and others). `a_pause_shorter_than_the_minimum_stays` and `an_empty_window_finds_nothing` pass against the stub.

- [ ] **Step 4: Implement `find_silences`**

Replace the stub function (keep its doc comment, extended as below) and add the constants above it:
```rust
/// Half the hold: the curve at a bucket is the loudest bucket this far
/// either side, so a voice's zero crossings never read as quiet.
const HOLD: f64 = 0.005;
/// A loud run shorter than this is a click, not speech.
const CLICK: f64 = 0.030;
/// Less than this left after padding is not worth a cut.
const LEAST: f64 = 0.020;

/// The spans of `from..to` (source seconds) that are silence by
/// `settings`, sorted and disjoint.
///
/// Past the end of the peaks is silence: a picture that outlasts its
/// sound loses the quiet tail too. A run touching `from` or `to` is only
/// padded at its inner end, so leading and trailing silence goes whole.
pub fn find_silences(
    peaks: &Peaks,
    from: f64,
    to: f64,
    settings: &SilenceSettings,
) -> Vec<(f64, f64)> {
    let rate = f64::from(peaks.buckets_per_second);
    let from = from.max(0.0);
    if rate <= 0.0 || to <= from {
        return Vec::new();
    }
    let first = (from * rate).floor() as usize;
    let last = ((to * rate).ceil() as usize).max(first);
    let threshold = 10f32.powf(settings.level_db / 20.0);
    let amplitude = |index: usize| -> f32 {
        if index < peaks.len() {
            peaks.max[index].max(-peaks.min[index])
        } else {
            0.0
        }
    };

    // Loud where the hold's loudest bucket reaches the level: a running
    // count of raw loud buckets, so each point asks its window in O(1).
    let raw: Vec<bool> = (first..last).map(|index| amplitude(index) >= threshold).collect();
    let n = raw.len();
    let mut count = vec![0usize; n + 1];
    for (index, loud) in raw.iter().enumerate() {
        count[index + 1] = count[index] + usize::from(*loud);
    }
    let hold = (HOLD * rate).round() as usize;
    let mut loud: Vec<bool> = (0..n)
        .map(|index| {
            let a = index.saturating_sub(hold);
            let b = (index + hold + 1).min(n);
            count[b] > count[a]
        })
        .collect();

    // A click is a loud run too short to be speech: quiet.
    let click = (CLICK * rate).round() as usize;
    let mut index = 0;
    while index < n {
        if !loud[index] {
            index += 1;
            continue;
        }
        let start = index;
        while index < n && loud[index] {
            index += 1;
        }
        if index - start < click {
            loud[start..index].fill(false);
        }
    }

    let at = |offset: usize| ((first + offset) as f64 / rate).clamp(from, to);
    let mut spans = Vec::new();
    let mut index = 0;
    while index < n {
        if loud[index] {
            index += 1;
            continue;
        }
        let start = index;
        while index < n && !loud[index] {
            index += 1;
        }
        let (mut a, mut b) = (at(start), at(index));
        if b - a < settings.min_pause {
            continue;
        }
        if start > 0 {
            a += settings.padding;
        }
        if index < n {
            b -= settings.padding;
        }
        if b - a >= LEAST {
            spans.push((a, b));
        }
    }
    spans
}
```

- [ ] **Step 5: Run the tests to see them pass**

Run: `wcargo test -p concat-media silence`
Expected: PASS, 12 tests. If `a_click_does_not_break_a_pause` expects `1.9` and gets `1.895`, check the arithmetic in its comment: the second stretch starts at 2.005, so the quiet run ends at 2.0 and padding gives 1.9.

- [ ] **Step 6: Lint**

Run: `wcargo clippy -p concat-media -- -D warnings`
Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
wgit add src/crates/concat-media/src/silence.rs src/crates/concat-media/src/lib.rs
wgit commit -m "\"feat(media): find where a recording goes quiet, off the waveform peaks\"" -m "\"Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>\""
```

---

### Task 2: The `RemoveClipRanges` command

**Files:**
- Create: `src/crates/concat-project/src/commands/cut.rs`
- Modify: `src/crates/concat-project/src/commands/mod.rs`, at the enum (after the `RemoveClips` variant, around line 543), `CommandError` (around line 705), `has_non_finite` (around line 912), the module list (line 21) and the dispatcher (around line 1030)
- Test: `src/crates/concat-project/src/lib.rs`, in `mod tests` after the ripple-delete tests (around line 400)

**Interfaces:**
- Consumes: `Command::SplitClips { clip_ids, time }`, `Command::RemoveClips { clip_ids, ripple: true }`, `super::apply`, `MIN_CLIP_DURATION` (1/60 s).
- Produces:
  ```rust
  Command::RemoveClipRanges { clip_id: String, ranges: Vec<(f64, f64)> }   // JSON: {"op":"removeClipRanges","clipId":..,"ranges":[[a,b],..]}
  CommandError::ClipGone        // "That clip no longer exists."
  CommandError::SpeedCurveCut   // "Silences can't be cut from a clip whose speed changes over time."
  CommandError::NothingLeft     // "That would remove the whole clip."
  ```

- [ ] **Step 1: Write the failing tests**

In `src/crates/concat-project/src/lib.rs`, inside `mod tests`, after `ripple_delete_leaves_a_clip_stacked_at_the_same_start_alone`, add:
```rust
    // ── remove ranges: what Remove Silences applies ──

    fn round(x: f64) -> f64 {
        (x * 1000.0).round() / 1000.0
    }

    /// `(start, duration, source_start)` of every clip on a track, in
    /// timeline order.
    fn pieces(editor: &Editor, track_id: &str) -> Vec<(f64, f64, f64)> {
        let mut out: Vec<(f64, f64, f64)> = editor
            .project()
            .active()
            .clips
            .iter()
            .filter(|clip| clip.track_id == track_id)
            .map(|clip| (round(clip.start), round(clip.duration), round(clip.source_start)))
            .collect();
        out.sort_by(|a, b| a.0.total_cmp(&b.0));
        out
    }

    fn tracks(editor: &Editor) -> (String, String) {
        let tracks = &editor.project().active().tracks;
        (tracks[0].id.clone(), tracks[1].id.clone())
    }

    #[test]
    fn removing_ranges_cuts_the_clip_and_closes_up_its_track() {
        let (mut editor, media_id, clip) = fixture();
        let (video, other) = tracks(&editor);
        let after = lane(&mut editor, &media_id, &video, &[10.0]);
        let elsewhere = lane(&mut editor, &media_id, &other, &[15.0]);

        let outcome = editor
            .apply(Command::RemoveClipRanges {
                clip_id: clip.clone(),
                ranges: vec![(5.0, 7.0), (2.0, 3.0)],
            })
            .expect("cuts");
        assert!(outcome.applied);
        assert_eq!(
            pieces(&editor, &video),
            vec![(0.0, 2.0, 0.0), (2.0, 2.0, 3.0), (4.0, 3.0, 7.0), (7.0, 10.0, 0.0)],
            "three pieces, and the clip after them pulled left by 3 s"
        );
        assert_eq!(start_of(&editor, &after[0]), 7.0);
        assert_eq!(start_of(&editor, &elsewhere[0]), 15.0, "another track stays");
    }

    #[test]
    fn removing_ranges_is_one_undo_step() {
        let (mut editor, _, clip) = fixture();
        let before = editor.project().active().clips.clone();
        editor
            .apply(Command::RemoveClipRanges {
                clip_id: clip,
                ranges: vec![(2.0, 3.0), (5.0, 7.0)],
            })
            .expect("cuts");
        assert_ne!(editor.project().active().clips, before);
        assert!(editor.undo());
        assert_eq!(editor.project().active().clips, before);
    }

    #[test]
    fn removing_ranges_cuts_the_detached_sound_in_step() {
        let (mut editor, _, clip) = fixture();
        let (video, sound_track) = tracks(&editor);
        editor
            .apply(Command::DetachAudio { clip_id: clip.clone() })
            .expect("detaches");
        editor
            .apply(Command::RemoveClipRanges {
                clip_id: clip,
                ranges: vec![(2.0, 3.0)],
            })
            .expect("cuts");
        let expected = vec![(0.0, 2.0, 0.0), (2.0, 7.0, 3.0)];
        assert_eq!(pieces(&editor, &video), expected);
        assert_eq!(pieces(&editor, &sound_track), expected);
    }

    #[test]
    fn naming_the_detached_sound_cuts_its_picture_too() {
        let (mut editor, _, clip) = fixture();
        let (video, sound_track) = tracks(&editor);
        editor
            .apply(Command::DetachAudio { clip_id: clip.clone() })
            .expect("detaches");
        let sound = editor
            .project()
            .active()
            .clips
            .iter()
            .find(|other| other.detached_from.as_deref() == Some(clip.as_str()))
            .expect("the sound")
            .id
            .clone();
        editor
            .apply(Command::RemoveClipRanges {
                clip_id: sound,
                ranges: vec![(2.0, 3.0)],
            })
            .expect("cuts");
        assert_eq!(pieces(&editor, &video), pieces(&editor, &sound_track));
        assert_eq!(pieces(&editor, &video).len(), 2);
    }

    #[test]
    fn removing_ranges_follows_a_trimmed_fast_clip() {
        let (mut editor, _, clip) = fixture();
        let video = tracks(&editor).0;
        editor
            .apply(Command::TrimClip {
                clip_id: clip.clone(),
                edge: TrimEdge::Start,
                delta: 2.0,
                ripple: false,
            })
            .expect("trims");
        editor
            .apply(Command::SetClipSpeed {
                clip_id: clip.clone(),
                speed: 2.0,
            })
            .expect("speeds up");
        // Now 4 s at 2x showing source 2..10 from timeline 2.
        let placed = pieces(&editor, &video);
        assert_eq!(placed, vec![(2.0, 4.0, 2.0)], "the set-up is what this test thinks");
        editor
            .apply(Command::RemoveClipRanges {
                clip_id: clip,
                ranges: vec![(4.0, 6.0)],
            })
            .expect("cuts");
        // Source 4..6 is timeline 3..4.
        assert_eq!(pieces(&editor, &video), vec![(2.0, 1.0, 2.0), (3.0, 2.0, 6.0)]);
    }

    #[test]
    fn fades_stay_at_the_ends_of_the_whole() {
        let (mut editor, _, clip) = fixture();
        editor
            .apply(Command::UpdateClip {
                clip_id: clip.clone(),
                patch: ClipPatch {
                    fade_in: Some(1.0),
                    fade_out: Some(1.0),
                    ..ClipPatch::default()
                },
            })
            .expect("fades");
        editor
            .apply(Command::RemoveClipRanges {
                clip_id: clip,
                ranges: vec![(4.0, 5.0)],
            })
            .expect("cuts");
        let mut clips: Vec<_> = editor.project().active().clips.iter().cloned().collect();
        clips.sort_by(|a, b| a.start.total_cmp(&b.start));
        assert_eq!((clips[0].fade_in, clips[0].fade_out), (1.0, 0.0));
        assert_eq!((clips[1].fade_in, clips[1].fade_out), (0.0, 1.0));
    }

    #[test]
    fn leading_silence_takes_the_head() {
        let (mut editor, _, clip) = fixture();
        let video = tracks(&editor).0;
        editor
            .apply(Command::RemoveClipRanges {
                clip_id: clip,
                ranges: vec![(0.0, 1.0)],
            })
            .expect("cuts");
        assert_eq!(pieces(&editor, &video), vec![(0.0, 9.0, 1.0)]);
    }

    #[test]
    fn ranges_closer_than_two_frames_merge_and_edges_snap() {
        let (mut editor, _, clip) = fixture();
        let video = tracks(&editor).0;
        editor
            .apply(Command::RemoveClipRanges {
                clip_id: clip,
                // 10 ms apart: one span. 10 ms from the end: to the end.
                ranges: vec![(2.0, 3.0), (3.01, 4.0), (9.0, 9.99)],
            })
            .expect("cuts");
        assert_eq!(pieces(&editor, &video), vec![(0.0, 2.0, 0.0), (2.0, 5.0, 4.0)]);
    }

    #[test]
    fn a_batch_cuts_two_clips_on_one_track() {
        let (mut editor, media_id, first) = fixture();
        let video = tracks(&editor).0;
        let second = lane(&mut editor, &media_id, &video, &[10.0]).remove(0);
        editor
            .apply(Command::Batch {
                commands: vec![
                    Command::RemoveClipRanges {
                        clip_id: first,
                        ranges: vec![(2.0, 3.0)],
                    },
                    Command::RemoveClipRanges {
                        clip_id: second,
                        ranges: vec![(2.0, 3.0)],
                    },
                ],
            })
            .expect("cuts both");
        assert_eq!(
            pieces(&editor, &video),
            vec![(0.0, 2.0, 0.0), (2.0, 7.0, 3.0), (9.0, 2.0, 0.0), (11.0, 7.0, 3.0)]
        );
    }

    #[test]
    fn removing_ranges_refuses_what_it_cannot_do() {
        let (mut editor, _, clip) = fixture();
        assert_eq!(
            editor.apply(Command::RemoveClipRanges {
                clip_id: "nope".to_owned(),
                ranges: vec![(1.0, 2.0)],
            }),
            Err(CommandError::ClipGone)
        );
        assert_eq!(
            editor.apply(Command::RemoveClipRanges {
                clip_id: clip.clone(),
                ranges: vec![(0.0, 10.0)],
            }),
            Err(CommandError::NothingLeft)
        );
        let nothing = editor
            .apply(Command::RemoveClipRanges {
                clip_id: clip.clone(),
                ranges: Vec::new(),
            })
            .expect("no ranges is fine");
        assert!(!nothing.applied);
        editor
            .apply(Command::SetClipSpeedCurve {
                clip_id: clip.clone(),
                curve: Some(vec![
                    crate::model::SpeedPoint { at: 0.0, speed: 1.0 },
                    crate::model::SpeedPoint { at: 1.0, speed: 2.0 },
                ]),
            })
            .expect("curves");
        assert_eq!(
            editor.apply(Command::RemoveClipRanges {
                clip_id: clip,
                ranges: vec![(1.0, 2.0)],
            }),
            Err(CommandError::SpeedCurveCut)
        );
        assert_eq!(
            editor.apply(Command::RemoveClipRanges {
                clip_id: "c1".to_owned(),
                ranges: vec![(f64::NAN, 2.0)],
            }),
            Err(CommandError::NotANumber)
        );
    }

    #[test]
    fn remove_clip_ranges_reads_from_json() {
        let command: Command = serde_json::from_value(json!({
            "op": "removeClipRanges",
            "clipId": "c1",
            "ranges": [[1.0, 2.0], [3.5, 4.0]]
        }))
        .expect("parses");
        assert_eq!(
            command,
            Command::RemoveClipRanges {
                clip_id: "c1".to_owned(),
                ranges: vec![(1.0, 2.0), (3.5, 4.0)],
            }
        );
    }
```
At the top of `mod tests`, extend the imports:
```rust
    use crate::commands::{ClipMove, ClipPatch, Command, CommandError, NewMedia, TrackFlag, TrimEdge};
```
(`CommandError` is new in that list; keep the others.) Also add, with the fades test:
```rust
    #[test]
    fn keys_stay_on_their_instant_of_the_picture() {
        let (mut editor, _, clip) = fixture();
        editor
            .apply(Command::SetClipKey {
                clip_id: clip.clone(),
                property: crate::model::KeyProperty::Scale,
                at: 0.8,
                value: 1.5,
                ease: crate::model::KeyEase::LINEAR,
            })
            .expect("keys");
        editor
            .apply(Command::RemoveClipRanges {
                clip_id: clip,
                ranges: vec![(2.0, 3.0)],
            })
            .expect("cuts");
        // Source 8 s is 5 s into the second piece, which is 7 s long.
        let mut clips: Vec<_> = editor.project().active().clips.iter().cloned().collect();
        clips.sort_by(|a, b| a.start.total_cmp(&b.start));
        assert!(
            clips[1]
                .keys
                .iter()
                .any(|key| key.property == crate::model::KeyProperty::Scale
                    && (key.at - 5.0 / 7.0).abs() < 0.01),
            "{:?}",
            clips[1].keys
        );
    }
```

- [ ] **Step 2: Run the tests to see them fail**

Run: `wcargo test -p concat-project remov`
Expected: FAIL to compile with "no variant named `RemoveClipRanges`".

- [ ] **Step 3: Add the variant**

In `src/crates/concat-project/src/commands/mod.rs`, directly after the `RemoveClips { .. },` variant, add:
```rust
    /// Takes spans out of a clip and closes up its track: what Remove
    /// Silences applies. `ranges` are source seconds; each is mapped onto
    /// the timeline through the clip's in-point and speed, clamped to the
    /// clip, and merged with any neighbour closer than two frames, and a
    /// span within two frames of an edge reaches it. The clip is split at
    /// each span and the pieces inside are ripple-removed, so keys, fades
    /// and transitions go as a hand cut takes them; clips later on the
    /// track move left, other tracks stay. A video and every sound
    /// detached from it are cut together, whichever is named. Refused for
    /// an unknown clip, a clip with a speed curve, or spans that leave
    /// nothing; no spans is a no-op.
    RemoveClipRanges {
        /// The clip to cut: a video, or a sound detached from one.
        clip_id: String,
        /// The spans to take out, in source seconds.
        ranges: Vec<(f64, f64)>,
    },
```

- [ ] **Step 4: Add the refusals**

In `CommandError`, after `NotANumber`, add:
```rust
    /// [`Command::RemoveClipRanges`] named a clip that is not there.
    #[error("That clip no longer exists.")]
    ClipGone,
    /// [`Command::RemoveClipRanges`] on a clip whose speed changes over
    /// its length: its source map is not a straight line to cut along.
    #[error("Silences can't be cut from a clip whose speed changes over time.")]
    SpeedCurveCut,
    /// [`Command::RemoveClipRanges`] would have taken out the whole clip.
    #[error("That would remove the whole clip.")]
    NothingLeft,
```

- [ ] **Step 5: Guard its numbers**

In `has_non_finite`, next to `Command::SplitClips { time, .. } => bad([*time]),`, add:
```rust
            Command::RemoveClipRanges { ranges, .. } => {
                bad(ranges.iter().flat_map(|(from, to)| [*from, *to]))
            }
```

- [ ] **Step 6: Route it**

In the module list after `mod clips;` add `mod cut;`. In `apply`, after the arm that routes to `clips::apply`, add:
```rust
        command @ Command::RemoveClipRanges { .. } => cut::apply(project, mint, command),
```

- [ ] **Step 7: Implement `cut.rs`**

Create `src/crates/concat-project/src/commands/cut.rs`:
```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Taking spans out of a clip: what Remove Silences applies.
//!
//! Built from the hand edits, a split at each end of a span and a ripple
//! remove of what is between, so a cut this way is exactly the cut a
//! person would make with the razor and a ripple delete, keys, fades and
//! transitions included.

use super::*;

/// Applies [`Command::RemoveClipRanges`]. Any other command is a routing
/// error.
pub(super) fn apply(
    project: &mut Project,
    mint: &mut IdMint,
    command: Command,
) -> Result<Outcome, CommandError> {
    let Command::RemoveClipRanges { clip_id, ranges } = command else {
        unreachable!("commands::apply routes only RemoveClipRanges here")
    };
    let group = {
        let timeline = project.active();
        let clip = timeline.clip(&clip_id).ok_or(CommandError::ClipGone)?;
        // The picture and every sound detached from it, whichever was named.
        let video = clip.detached_from.clone().unwrap_or_else(|| clip.id.clone());
        let group: Vec<Clip> = timeline
            .clips
            .iter()
            .filter(|other| {
                other.id == video || other.detached_from.as_deref() == Some(video.as_str())
            })
            .map(|other| Clip::clone(other))
            .collect();
        if group.iter().any(|member| member.speed_curve.is_some()) {
            return Err(CommandError::SpeedCurveCut);
        }
        group
    };

    // All or nothing, as a batch is: a refusal part way leaves no half cut.
    let mut staged = project.clone();
    let mut applied = false;
    for member in &group {
        let spans = spans_on(member, &ranges);
        if spans.is_empty() {
            continue;
        }
        let end = member.start + member.duration;
        if spans[0].0 <= member.start && spans[0].1 >= end {
            return Err(CommandError::NothingLeft);
        }
        // From the last span back, so what is still to cut never moves:
        // the named clip is always the piece left of the next cut.
        for (from, to) in spans.into_iter().rev() {
            let id = member.id.clone();
            super::apply(
                &mut staged,
                mint,
                Command::SplitClips {
                    clip_ids: vec![id.clone()],
                    time: to,
                },
            )?;
            let middle = super::apply(
                &mut staged,
                mint,
                Command::SplitClips {
                    clip_ids: vec![id.clone()],
                    time: from,
                },
            )?
            .created_id
            .unwrap_or(id);
            super::apply(
                &mut staged,
                mint,
                Command::RemoveClips {
                    clip_ids: vec![middle],
                    ripple: true,
                },
            )?;
            applied = true;
        }
    }
    *project = staged;
    Ok(Outcome {
        created_id: None,
        applied,
    })
}

/// `ranges` (source seconds) as timeline spans on `clip`: mapped through
/// its in-point and speed, clamped to it, sorted, merged where they come
/// closer than two frames, reaching an edge they come within two frames
/// of, and never shorter than two frames, so no split falls inside the
/// minimum a clip may be.
fn spans_on(clip: &Clip, ranges: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let end = clip.start + clip.duration;
    let at = |source: f64| clip.start + (source - clip.source_start) / clip.speed;
    let mut spans: Vec<(f64, f64)> = ranges
        .iter()
        .map(|(a, b)| (at(a.min(*b)).max(clip.start), at(a.max(*b)).min(end)))
        .filter(|(from, to)| to > from)
        .collect();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let snap = 2.0 * MIN_CLIP_DURATION;
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (mut from, mut to) in spans {
        if from - clip.start < snap {
            from = clip.start;
        }
        if end - to < snap {
            to = end;
        }
        match merged.last_mut() {
            Some(last) if from - last.1 < snap => last.1 = last.1.max(to),
            _ => merged.push((from, to)),
        }
    }
    merged.retain(|(from, to)| to - from >= snap);
    merged
}
```

- [ ] **Step 8: Run the tests to see them pass**

Run: `wcargo test -p concat-project`
Expected: PASS, every test, the twelve new ones included. If `fades_stay_at_the_ends_of_the_whole` fails on the tail's `fade_in`, re-read `SplitClips` (`commands/clips.rs:251`): the tail sets `fade_in = 0` and the head `fade_out = 0`, which is what the test expects.

- [ ] **Step 9: Lint**

Run: `wcargo clippy -p concat-project -- -D warnings`
Expected: no warnings.

- [ ] **Step 10: Commit**

```bash
wgit add src/crates/concat-project/src/commands/cut.rs src/crates/concat-project/src/commands/mod.rs src/crates/concat-project/src/lib.rs
wgit commit -m "\"feat(project): RemoveClipRanges takes spans out of a clip and closes up its track\"" -m "\"Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>\""
```

---

### Task 3: End to end through the export

**Files:**
- Test: `src/crates/concat-host/tests/export.rs` (a new test after `one_clip_exports_whole_at_every_rate`, around line 700)

**Interfaces:**
- Consumes: `concat_media::silence::{find_silences, SilenceSettings}` (Task 1), `Command::RemoveClipRanges` (Task 2), and the file's own `Scratch`, `Sources` (`peek`: 6 s, tone in the odd seconds), `Studio`, `Exported { frames, fps, .. }`, `expect_tone`, `expect_second`.
- Produces: nothing new.

- [ ] **Step 1: Write the test**

```rust
/// Remove Silences end to end: the pauses read off the clock's waveform
/// are its even seconds, and the cut plays the odd seconds back to back.
#[test]
fn removing_silences_leaves_only_the_sound() {
    use concat_media::silence::{SilenceSettings, find_silences};

    let scratch = Scratch::new("silences");
    let sources = Sources::make(scratch.path());
    let mut studio = Studio::new(scratch.path(), "Silences", video(WIDTH, HEIGHT, 30, 1));
    let peek = studio.import(&sources.peek);
    let clip = studio
        .apply(Command::AddClipAtFirstFree {
            media_id: peek,
            start: 0.0,
        })
        .expect("placed");

    let peaks = media::peaks(sources.peek.to_str().expect("utf-8"), None, None).expect("peaks");
    let settings = SilenceSettings {
        padding: 0.0,
        ..SilenceSettings::default()
    };
    let ranges = find_silences(&peaks, 0.0, 6.0, &settings);
    assert_eq!(ranges.len(), 3, "the even seconds: {ranges:?}");
    for ((from, to), second) in ranges.iter().zip([0.0, 2.0, 4.0]) {
        assert!(
            (from - second).abs() < 0.05 && (to - (second + 1.0)).abs() < 0.05,
            "{ranges:?}"
        );
    }

    studio.apply(Command::RemoveClipRanges {
        clip_id: clip,
        ranges,
    });
    let exported = studio.export_at("silences removed", None);
    let seconds = exported.frames.len() as f64 / exported.fps;
    assert!((seconds - 3.0).abs() < 0.1, "three seconds of tone, not {seconds}s");
    exported.expect_tone(0.5);
    exported.expect_tone(1.5);
    exported.expect_tone(2.5);
    exported.expect_second(0.5, 1);
    exported.expect_second(1.5, 3);
    exported.expect_second(2.5, 5);
}
```

- [ ] **Step 2: Run it**

Run: `wcargo test -p concat-host --test export removing_silences`
Expected: PASS. Tasks 1 and 2 already exist, so this test holds them together rather than driving new code. If the range assertion fails, print `ranges` (the message does) and compare with the peaks: AAC may start the tone a few milliseconds late, and 0.05 s is the tolerance for that.

- [ ] **Step 3: Run the whole export suite once**

Run: `wcargo test -p concat-host --test export`
Expected: PASS, nothing else broken.

- [ ] **Step 4: Commit**

```bash
wgit add src/crates/concat-host/tests/export.rs
wgit commit -m "\"test(export): silences read off a clip's waveform come out of its export\"" -m "\"Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>\""
```

---

### Task 4: The sheet's logic, as a pure plan

**Files:**
- Create: `src/crates/concat/src/panes/silence.rs` (the pure part only in this task)
- Modify: `src/crates/concat/src/panes/mod.rs` (add `pub mod silence;` after `pub mod settings;`)
- Test: in `silence.rs`, `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: `concat_media::silence::{SilenceSettings, find_silences}`, `concat_media::{Peaks, Pyramid}` (`Pyramid::of(Peaks)`, `.finest()`), `concat_project::Command::RemoveClipRanges`.
- Produces (used by Task 5):
  ```rust
  pub enum Skip { NoSound, SpeedCurve, Loading }
  pub struct Placement { pub row: i32, pub start: f64, pub duration: f64, pub source_start: f64, pub speed: f64 }
  pub struct Subject { pub clip_id: String, pub heard: Placement, pub members: Vec<Placement>, pub peaks: Result<Arc<Pyramid>, Skip> }
  pub struct Plan { pub commands: Vec<Command>, pub shades: Vec<(i32, f64, f64)>, pub pauses: usize, pub removed: f64, pub before: f64, pub skipped: Vec<Skip>, pub whole: bool }
  pub fn plan(subjects: &[Subject], settings: &SilenceSettings) -> Plan;
  pub fn clock(seconds: f64) -> String;
  pub fn leaders(clips: &[(String, Option<String>)], selection: &[String]) -> Vec<String>;
  ```
  `leaders` takes `(clip id, detached_from)` pairs and returns one id per group in selection order: the video's id, or the sound's own id when its video is gone.

- [ ] **Step 1: Write the failing tests**

Create `src/crates/concat/src/panes/silence.rs`:
```rust
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
pub fn plan(_subjects: &[Subject], _settings: &SilenceSettings) -> Plan {
    Plan::default()
}

/// A length for the summary: "45 s", "1 m 32 s", "0.4 s".
pub fn clock(_seconds: f64) -> String {
    String::new()
}

/// One id per selected group, in selection order: a video and the sounds
/// detached from it are one group, named by the video, or by the sound
/// when its video is gone. `clips` is every clip as `(id, detached_from)`.
pub fn leaders(_clips: &[(String, Option<String>)], _selection: &[String]) -> Vec<String> {
    Vec::new()
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
```
In `src/crates/concat/src/panes/mod.rs`, add `pub mod silence;` after `pub mod settings;`.

- [ ] **Step 2: Run the tests to see them fail**

Run: `wcargo test -p concat --lib silence`
Expected: FAIL. The assertions on the stubs fail, and `a_skipped_subject_is_counted_and_cut_nowhere` fails because `skipped` is empty. There will be `dead_code` warnings for items Task 5 will use; they go away in Task 5.

- [ ] **Step 3: Implement**

Replace the three stubs:
```rust
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
```

- [ ] **Step 4: Run the tests to see them pass**

Run: `wcargo test -p concat --lib silence`
Expected: PASS, 6 tests.

- [ ] **Step 5: Commit**

```bash
wgit add src/crates/concat/src/panes/silence.rs src/crates/concat/src/panes/mod.rs
wgit commit -m "\"feat(window): Remove Silences' plan: pauses, shades and one cut per clip group\"" -m "\"Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>\""
```

---

### Task 5: The sheet, the tray button and the wiring

**Files:**
- Modify: `src/crates/concat/src/panes/silence.rs` (the pane: state, `SilenceMsg`, `update`, `data`, `subjects`, `blocked`)
- Modify: `src/crates/concat/src/panes/mod.rs` (the `Msg` enum)
- Modify: `src/crates/concat/src/prefs.rs` (three fields)
- Modify: `src/crates/concat/src/studio.rs` (field, default, `peaks_of`, routing near line 6733, publishing near lines 7312 and 8826, refresh on peaks near line 2913)
- Modify: `src/crates/concat/src/lib.rs` (callbacks near line 1435, `use` near line 50)
- Create: `src/crates/concat/ui/dialogs/silence.slint`
- Modify: `src/crates/concat/ui/app.slint`, `ui/editor.slint`, `ui/workspace/seat.slint`, `ui/workspace/timeline-pane.slint`, `ui/timeline/tray.slint`
- Modify: all 14 files in `src/crates/concat/locales/`
- Test: the pane's pure tests from Task 4, the locale test (`wcargo test -p concat --lib`), `scripts/locales.py --check`, and the app by hand

**Interfaces:**
- Consumes: Task 4's `plan`, `Plan`, `Subject`, `Placement`, `Skip`, `clock`, `leaders`. From the studio: `selection: Vec<String>`, `clip(&str) -> Option<&Clip>`, `timeline() -> &Timeline`, `row_of(&str) -> i32`, `clip_has_sound(&Clip) -> bool`, `apply(Command) -> Option<String>`, `notify(&str, bool)`, `prefs`, `host.dirs`, `peaks: HashMap<String, Arc<Pyramid>>`, and the file-private `art_key(&str, Option<u32>) -> String`.
- Produces (used by Task 6): `SilencePane::shades(&self) -> Vec<(i32, f64, f64)>`.

- [ ] **Step 1: The strings**

Save this as `/tmp/claude-1000/silence_strings.py` (a scratch file, not in the repo) and run `python3 /tmp/claude-1000/silence_strings.py`. It appends the keys to each locale, keeping each file's CRLF and checking it still parses:
```python
import json, pathlib
root = pathlib.Path("/mnt/f/Work/Playground/Concat/src/crates/concat/locales")
KEYS = ["tray.removeSilences", "silence.detection", "silence.silenceLevel", "silence.minimumPause",
        "silence.padding", "silence.summary", "silence.noPausesFound", "silence.wholeClipBelowLevel",
        "silence.clipsSkipped", "silence.reasonNoSound", "silence.reasonSpeedCurve", "silence.reasonLoading",
        "silence.selectClipWithSound", "silence.cannotRemove", "silence.removePauses", "silence.removedPauses"]
T = {
 "en": ["Remove Silences", "Detection", "Silence level", "Minimum pause", "Padding",
        "{0} pauses · removes {1} · {2} → {3}", "No pauses found at this level",
        "The whole clip is below this level", "{0} clips skipped: {1}", "no sound",
        "its speed changes over time", "the sound is still being read",
        "Select a clip with sound to remove its silences", "Can't remove silences: {0}",
        "Remove pauses", "Removed {0} pauses ({1})"],
 "de": ["Stille entfernen", "Erkennung", "Stillepegel", "Minimale Pause", "Puffer",
        "{0} Pausen · entfernt {1} · {2} → {3}", "Bei diesem Pegel keine Pausen gefunden",
        "Der ganze Clip liegt unter diesem Pegel", "{0} Clips übersprungen: {1}", "kein Ton",
        "seine Geschwindigkeit ändert sich", "der Ton wird noch gelesen",
        "Wähle einen Clip mit Ton, um seine Stille zu entfernen", "Stille kann nicht entfernt werden: {0}",
        "Pausen entfernen", "{0} Pausen entfernt ({1})"],
 "es": ["Quitar silencios", "Detección", "Nivel de silencio", "Pausa mínima", "Margen",
        "{0} pausas · quita {1} · {2} → {3}", "No se encontraron pausas con este nivel",
        "Todo el clip está por debajo de este nivel", "{0} clips omitidos: {1}", "sin sonido",
        "su velocidad cambia con el tiempo", "el sonido aún se está leyendo",
        "Selecciona un clip con sonido para quitar sus silencios", "No se pueden quitar los silencios: {0}",
        "Quitar pausas", "Se quitaron {0} pausas ({1})"],
 "fa": ["حذف سکوت‌ها", "تشخیص", "سطح سکوت", "حداقل مکث", "حاشیه",
        "{0} مکث · حذف {1} · {2} → {3}", "در این سطح مکثی پیدا نشد",
        "کل کلیپ زیر این سطح است", "{0} کلیپ رد شد: {1}", "بدون صدا",
        "سرعت آن در طول زمان تغییر می‌کند", "صدا هنوز در حال خواندن است",
        "یک کلیپ صدادار انتخاب کنید تا سکوت‌هایش حذف شود", "حذف سکوت‌ها ممکن نیست: {0}",
        "حذف مکث‌ها", "{0} مکث حذف شد ({1})"],
 "fr": ["Supprimer les silences", "Détection", "Niveau de silence", "Pause minimale", "Marge",
        "{0} pauses · retire {1} · {2} → {3}", "Aucune pause trouvée à ce niveau",
        "Tout le clip est sous ce niveau", "{0} clips ignorés : {1}", "pas de son",
        "sa vitesse varie dans le temps", "le son est encore en cours de lecture",
        "Sélectionnez un clip avec du son pour en supprimer les silences",
        "Impossible de supprimer les silences : {0}", "Supprimer les pauses", "{0} pauses supprimées ({1})"],
 "hr": ["Ukloni tišine", "Otkrivanje", "Razina tišine", "Najkraća stanka", "Odmak",
        "{0} stanki · uklanja {1} · {2} → {3}", "Na ovoj razini nema stanki",
        "Cijeli isječak je ispod ove razine", "Preskočeno isječaka: {0} ({1})", "nema zvuka",
        "brzina mu se mijenja tijekom vremena", "zvuk se još čita",
        "Odaberite isječak sa zvukom da biste uklonili tišine", "Tišine se ne mogu ukloniti: {0}",
        "Ukloni stanke", "Uklonjeno stanki: {0} ({1})"],
 "it": ["Rimuovi silenzi", "Rilevamento", "Livello di silenzio", "Pausa minima", "Margine",
        "{0} pause · rimuove {1} · {2} → {3}", "Nessuna pausa trovata a questo livello",
        "L'intera clip è sotto questo livello", "{0} clip saltate: {1}", "nessun suono",
        "la sua velocità cambia nel tempo", "il suono è ancora in lettura",
        "Seleziona una clip con audio per rimuoverne i silenzi", "Impossibile rimuovere i silenzi: {0}",
        "Rimuovi pause", "Rimosse {0} pause ({1})"],
 "ja": ["無音を削除", "検出", "無音レベル", "最小の間", "余白",
        "{0} 個の間 · {1} を削除 · {2} → {3}", "このレベルでは間が見つかりません",
        "クリップ全体がこのレベルを下回っています", "{0} 個のクリップをスキップ: {1}", "音声なし",
        "速度が時間とともに変化します", "音声を読み込み中です",
        "無音を削除するには音声のあるクリップを選択してください", "無音を削除できません: {0}",
        "間を削除", "{0} 個の間を削除しました（{1}）"],
 "ko": ["무음 제거", "감지", "무음 레벨", "최소 멈춤", "여유",
        "멈춤 {0}개 · {1} 제거 · {2} → {3}", "이 레벨에서 멈춤을 찾지 못했습니다",
        "클립 전체가 이 레벨보다 낮습니다", "클립 {0}개 건너뜀: {1}", "소리 없음",
        "속도가 시간에 따라 바뀝니다", "소리를 아직 읽는 중입니다",
        "무음을 제거하려면 소리가 있는 클립을 선택하세요", "무음을 제거할 수 없습니다: {0}",
        "멈춤 제거", "멈춤 {0}개 제거됨 ({1})"],
 "pt-BR": ["Remover silêncios", "Detecção", "Nível de silêncio", "Pausa mínima", "Margem",
        "{0} pausas · remove {1} · {2} → {3}", "Nenhuma pausa encontrada neste nível",
        "O clipe inteiro está abaixo deste nível", "{0} clipes ignorados: {1}", "sem som",
        "a velocidade muda ao longo do tempo", "o som ainda está sendo lido",
        "Selecione um clipe com som para remover os silêncios", "Não é possível remover os silêncios: {0}",
        "Remover pausas", "{0} pausas removidas ({1})"],
 "ru": ["Удалить тишину", "Обнаружение", "Уровень тишины", "Минимальная пауза", "Запас",
        "Пауз: {0} · удаляет {1} · {2} → {3}", "На этом уровне пауз не найдено",
        "Весь клип ниже этого уровня", "Пропущено клипов: {0} — {1}", "нет звука",
        "скорость меняется со временем", "звук ещё считывается",
        "Выберите клип со звуком, чтобы удалить тишину", "Не удаётся удалить тишину: {0}",
        "Удалить паузы", "Удалено пауз: {0} ({1})"],
 "tr": ["Sessizlikleri kaldır", "Algılama", "Sessizlik düzeyi", "En kısa duraklama", "Pay",
        "{0} duraklama · {1} kaldırır · {2} → {3}", "Bu düzeyde duraklama bulunamadı",
        "Klibin tamamı bu düzeyin altında", "{0} klip atlandı: {1}", "ses yok",
        "hızı zamanla değişiyor", "ses hâlâ okunuyor",
        "Sessizlikleri kaldırmak için sesli bir klip seçin", "Sessizlikler kaldırılamıyor: {0}",
        "Duraklamaları kaldır", "{0} duraklama kaldırıldı ({1})"],
 "zh-Hans": ["移除静音", "检测", "静音电平", "最短停顿", "留白",
        "{0} 处停顿 · 移除 {1} · {2} → {3}", "在此电平下未找到停顿",
        "整个片段都低于此电平", "已跳过 {0} 个片段：{1}", "没有声音",
        "其速度随时间变化", "仍在读取声音",
        "选择一个有声音的片段以移除其静音", "无法移除静音：{0}",
        "移除停顿", "已移除 {0} 处停顿（{1}）"],
 "zh-TW": ["移除靜音", "偵測", "靜音音量", "最短停頓", "留白",
        "{0} 處停頓 · 移除 {1} · {2} → {3}", "在此音量下找不到停頓",
        "整個片段都低於此音量", "已略過 {0} 個片段：{1}", "沒有聲音",
        "其速度隨時間變化", "仍在讀取聲音",
        "選取一個有聲音的片段以移除其靜音", "無法移除靜音：{0}",
        "移除停頓", "已移除 {0} 處停頓（{1}）"],
}
assert set(T) == {p.stem for p in root.glob("*.json")}, "every shipped locale, no more"
for code, words in T.items():
    assert len(words) == len(KEYS), code
    path = root / f"{code}.json"
    raw = path.read_bytes().decode("utf-8")
    nl = "\r\n" if "\r\n" in raw else "\n"
    existing = json.loads(raw)
    body = raw.rstrip()
    assert body.endswith("}"), code
    body = body[:-1].rstrip()
    lines = "".join(
        f",{nl}  {json.dumps(k)}: {json.dumps(v, ensure_ascii=False)}"
        for k, v in zip(KEYS, words) if k not in existing
    )
    path.write_bytes((body + lines + nl + "}" + nl).encode("utf-8"))
    json.loads(path.read_text(encoding="utf-8"))
print("ok")
```
Expected output: `ok`.

- [ ] **Step 2: Remember the settings**

In `src/crates/concat/src/prefs.rs`, in `Preferences`, after `preview_axis_audio`, add:
```rust
    /// Remove Silences' level as last used, in dBFS. `None` is -40.
    pub silence_level_db: Option<f32>,
    /// Remove Silences' minimum pause as last used, in seconds. `None` is 0.5.
    pub silence_min_pause: Option<f64>,
    /// Remove Silences' padding as last used, in seconds. `None` is 0.1.
    pub silence_padding: Option<f64>,
```

- [ ] **Step 3: The pane**

Append to `src/crates/concat/src/panes/silence.rs`, above `#[cfg(test)]`, and extend its `use` lines:
```rust
use concat_project::model::Clip;

use crate::i18n::{t, tf};
use crate::prefs::Preferences;
use crate::studio::Studio;
use crate::ui::SilenceSheetData;

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
```
If `studio.prefs` or `studio.host` is not `pub(crate)` or wider, the compiler will say so. `panes/speech.rs:314` already writes `studio.prefs.tts_voice` and calls `studio.prefs.save(&studio.host.dirs)`, so both are reachable.

- [ ] **Step 4: Route messages**

In `src/crates/concat/src/panes/mod.rs`, add `pub mod silence;` if Task 4 did not, and in `Msg`, after `Captions(..)`, add:
```rust
    /// To the Remove Silences sheet.
    Silence(silence::SilenceMsg),
```

- [ ] **Step 5: The studio**

In `src/crates/concat/src/studio.rs`:

1. Next to `pub captions: crate::panes::captions::CaptionsPane,` (around line 900), add:
   ```rust
       pub silence: crate::panes::silence::SilencePane,
   ```
   and next to `captions: crate::panes::captions::CaptionsPane::default(),` (around line 2072), add:
   ```rust
               silence: crate::panes::silence::SilencePane::default(),
   ```
2. In `Models` (line 556), after `pub guides: ...`, add:
   ```rust
       pub silence_shades: Rc<VecModel<SilenceShadeData>>,
   ```
   and in its construction (around line 645), after `guides: Rc::new(VecModel::default()),`, add:
   ```rust
               silence_shades: Rc::new(VecModel::default()),
   ```
3. After `fn wave` (around line 3080), add:
   ```rust
       /// The waveform of the sound `clip` plays, when it has been read.
       pub(crate) fn peaks_of(&self, clip: &Clip) -> Option<Arc<Pyramid>> {
           self.peaks.get(&art_key(&clip.media_id, clip.audio_stream)).cloned()
       }
   ```
   (Use the same `Clip`, `Arc` and `Pyramid` paths the file already uses for `peaks`. `peaks` is `HashMap<String, Arc<Pyramid>>` at line 740.)
4. In `handle`, after the `Msg::Captions` arm (around line 6733), add:
   ```rust
               crate::panes::Msg::Silence(msg) => {
                   let mut pane = std::mem::take(&mut self.silence);
                   pane.update(msg, self);
                   self.silence = pane;
               }
   ```
5. Where `sync(&models.guides, self.stage_guides.clone());` is (around line 7219), add:
   ```rust
           sync(
               &models.silence_shades,
               self.silence
                   .shades()
                   .into_iter()
                   .map(|(row, start, length)| SilenceShadeData {
                       row,
                       start: start as f32,
                       length: length as f32,
                   })
                   .collect(),
           );
   ```
6. Next to `editor.set_merge_blocked_because(...)` (around line 7312), add the same shape:
   ```rust
           editor.set_silence_blocked_because(
               crate::panes::silence::blocked(self)
                   .map(SharedString::from)
                   .unwrap_or_default(),
           );
   ```
7. After `app.set_captions(self.captions.data(self));` (around line 8826), add:
   ```rust
           app.set_silence(self.silence.data());
   ```
8. Where a waveform arrives (around line 2913, just after `studio.peaks.insert(key, peaks);` and its `waves` retain), add:
   ```rust
                           if studio.silence.open {
                               studio.handle(crate::panes::Msg::Silence(
                                   crate::panes::silence::SilenceMsg::Refresh,
                               ));
                           }
   ```

- [ ] **Step 6: The callbacks**

In `src/crates/concat/src/lib.rs`, next to `use panes::captions::CaptionsMsg;` (line 50), add `use panes::silence::SilenceMsg;`. Next to `editor.set_stage_guides(ModelRc::from(models.guides.clone()));` (line 207), add:
```rust
        editor.set_silence_shades(ModelRc::from(models.silence_shades.clone()));
```
After the captions callbacks (around line 1461), add:
```rust
    editor.on_silences(on_window!(|state| {
        state.handle(Msg::Silence(SilenceMsg::Open));
    }));
    app.on_silence_closed(on_window!(|state| {
        state.handle(Msg::Silence(SilenceMsg::Close));
    }));
    app.on_silence_level_changed(on_window!(|state, value: f32| {
        state.handle(Msg::Silence(SilenceMsg::LevelChanged(value)));
    }));
    app.on_silence_min_pause_changed(on_window!(|state, value: f32| {
        state.handle(Msg::Silence(SilenceMsg::MinPauseChanged(value)));
    }));
    app.on_silence_padding_changed(on_window!(|state, value: f32| {
        state.handle(Msg::Silence(SilenceMsg::PaddingChanged(value)));
    }));
    app.on_silence_apply(on_window!(|state| {
        state.handle(Msg::Silence(SilenceMsg::Apply));
    }));
```

- [ ] **Step 7: The sheet**

Create `src/crates/concat/ui/dialogs/silence.slint`:
```slint
// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import { I18n } from "../i18n.slint";
import { Theme } from "../theme/theme.slint";
import { Glyph } from "../icons.slint";
import { Button, DialogRow, DialogSection, Modal } from "../primitives/modal.slint";
import { NumberField } from "../primitives/number-field.slint";

// Remove Silences: what the tray's tool opens over the selected clips.
//
// Three numbers say what a pause is. The line under them says what they
// find, recomputed in Rust as they move, and the lanes shade the same
// spans while the sheet is open. One button cuts them all, one undo step.

export struct SilenceSheetData {
    open: bool,
    /// dBFS, -60..-20
    level: float,
    /// seconds, 0.1..3
    min-pause: float,
    /// seconds, 0..0.5
    padding: float,
    /// what the settings find, or why nothing will be cut
    summary: string,
    /// the clips left out and why; empty when none are
    skipped: string,
    /// Apply has something to cut
    ready: bool,
}

export component SilenceDialog inherits Modal {
    in property <SilenceSheetData> data;

    callback level-changed(value: float);
    callback min-pause-changed(value: float);
    callback padding-changed(value: float);
    callback apply();

    open: root.data.open;
    title: I18n.t("tray.removeSilences");
    glyph: Glyph.waveform;
    surface-width: 420px;

    // The fields own their numbers while they are dragged. Rust's copy comes
    // back on every publish as the same number, so they are re-seeded from
    // it rather than bound to it, the way the captions sheet holds its draft.
    property <float> level;
    property <float> min-pause;
    property <float> padding;
    function seed() {
        root.level = root.data.level;
        root.min-pause = root.data.min-pause;
        root.padding = root.data.padding;
    }
    changed data => { root.seed(); }
    init => { root.seed(); }

    VerticalLayout {
        DialogSection {
            first: true;
            title: I18n.t("silence.detection");

            DialogRow {
                label: I18n.t("silence.silenceLevel");
                NumberField {
                    value <=> root.level;
                    minimum: -60;
                    maximum: -20;
                    step: 1;
                    precision: 0;
                    unit: "dB";
                    changed(value) => { root.level-changed(value); }
                }
            }

            DialogRow {
                label: I18n.t("silence.minimumPause");
                NumberField {
                    value <=> root.min-pause;
                    minimum: 0.1;
                    maximum: 3;
                    step: 0.1;
                    precision: 1;
                    unit: "s";
                    changed(value) => { root.min-pause-changed(value); }
                }
            }

            DialogRow {
                label: I18n.t("silence.padding");
                NumberField {
                    value <=> root.padding;
                    minimum: 0;
                    maximum: 0.5;
                    step: 0.05;
                    precision: 2;
                    unit: "s";
                    changed(value) => { root.padding-changed(value); }
                }
            }

            Text {
                text: root.data.summary;
                color: root.data.ready ? Theme.fg : Theme.fg-muted;
                font-size: Theme.fs-sm;
                wrap: word-wrap;
            }

            if root.data.skipped != "": Text {
                text: root.data.skipped;
                color: Theme.fg-dim;
                font-size: Theme.fs-sm;
                wrap: word-wrap;
            }
        }

        Rectangle {
            height: 1px;
            background: Theme.line;
        }

        HorizontalLayout {
            padding: 14px;
            spacing: 8px;

            Button {
                label: I18n.t("common.cancel");
                glyph: Glyph.close;
                ghost: true;
                clicked => { root.closed(); }
            }

            Button {
                horizontal-stretch: 1;
                primary: true;
                large: true;
                glyph: Glyph.razor;
                label: I18n.t("silence.removePauses");
                enabled: root.data.ready;
                clicked => { root.apply(); }
            }
        }
    }
}
```

- [ ] **Step 8: The app root**

In `src/crates/concat/ui/app.slint`:
1. After `import { CaptionsDialog, CaptionsSheetData } from "dialogs/captions.slint";`, add:
   `import { SilenceDialog, SilenceSheetData } from "dialogs/silence.slint";`
2. In the `import { ... } from "timeline/model.slint";` list (the one ending at line 47), add `SilenceShadeData,`. In the `export { ... }` list, add `SilenceShadeData,` after `StageGuideData,` and `SilenceSheetData,` after `CaptionsSheetData,`.
3. Make `sheet-open` (line 112) read:
   ```slint
       property <bool> sheet-open: root.export.open || root.settings.open
           || root.project-sheet.open || root.captions.open || root.speech.open
           || root.silence.open || root.start.composing;
   ```
4. After the captions properties and callbacks (after `callback captions-cancel();`), add:
   ```slint
       /// The Remove Silences sheet: three numbers and what they find.
       in property <SilenceSheetData> silence;
       callback silence-closed();
       callback silence-level-changed(value: float);
       callback silence-min-pause-changed(value: float);
       callback silence-padding-changed(value: float);
       callback silence-apply();
   ```
5. After the `CaptionsDialog { ... }` element, add:
   ```slint
       SilenceDialog {
           x: 0;
           y: 0;
           width: root.width;
           height: root.height;
           data: root.silence;

           closed => { root.silence-closed(); }
           level-changed(value) => { root.silence-level-changed(value); }
           min-pause-changed(value) => { root.silence-min-pause-changed(value); }
           padding-changed(value) => { root.silence-padding-changed(value); }
           apply => { root.silence-apply(); }
       }
   ```

- [ ] **Step 9: The shade type, the Editor global and the tray button**

1. `src/crates/concat/ui/timeline/model.slint`: after `EffectSpanData`, add:
   ```slint
   /// A span the open Remove Silences sheet would take out, as the lanes
   /// shade it. Seconds on the timeline.
   export struct SilenceShadeData {
       row: int,
       start: float,
       length: float,
   }
   ```
2. `src/crates/concat/ui/editor.slint`: add `SilenceShadeData` to its `import { ... } from "timeline/model.slint"` list (the list that brings in `DropData`). After `in-out property <string> merge-blocked-because;`, add:
   ```slint
       /// Remove Silences: "" when the selection can be worked on, otherwise
       /// why not, which becomes the tray button's label.
       in-out property <string> silence-blocked-because;
       /// The spans the open Remove Silences sheet would take out.
       in-out property <[SilenceShadeData]> silence-shades;
   ```
   After `callback speak();`, add:
   ```slint
       /// The tray's Remove Silences tool; opens its sheet.
       callback silences();
   ```
3. `src/crates/concat/ui/timeline/tray.slint`: after `in property <string> merge-blocked-because;`, add `in property <string> silence-blocked-because;`. After `callback speak();`, add `callback silences();`. After the Text to Speech `TrayButton { ... }`, add:
   ```slint
           TrayButton {
               glyph: Glyph.waveform;
               label: root.silence-blocked-because == "" ? I18n.t("tray.removeSilences")
                   : root.silence-blocked-because;
               icon-size: 16px;
               enabled: root.silence-blocked-because == "";
               clicked => { root.silences(); }
           }
   ```
4. `src/crates/concat/ui/workspace/timeline-pane.slint`: add `SilenceShadeData` to its `import { ... }` from the timeline model (the file that declares `in property <DropData> drop;` imports `DropData` from there). After `in property <string> merge-blocked-because;` (line 48), add:
   ```slint
       in property <string> silence-blocked-because;
       in property <[SilenceShadeData]> silence-shades;
   ```
   and next to its `callback captions();`, add `callback silences();`. On the `TimelineTray { ... }` element (line 301), add `silence-blocked-because: root.silence-blocked-because;` next to `merge-blocked-because: ...` and `silences => { root.silences(); }` next to `captions => { root.captions(); }`.
5. `src/crates/concat/ui/workspace/seat.slint`: on the timeline pane element, next to `merge-blocked-because: Editor.merge-blocked-because;` (line 369), add `silence-blocked-because: Editor.silence-blocked-because;` and `silence-shades: Editor.silence-shades;`. Next to `captions => { Editor.captions(); }` (line 400), add `silences => { Editor.silences(); }`.

`silence-shades` reaches the pane here but is only drawn in Task 6.

- [ ] **Step 10: Build, test, check the strings**

Run: `wcargo build --profile quick -p concat --bin concat`
Expected: builds. Slint errors name the file and line. The usual cause is a property missing in one link of the chain Editor → seat → timeline-pane → tray.

Run: `wcargo test -p concat --lib`
Expected: PASS, including `every_shipped_locale_parses_names_itself_and_keys_off_the_inventory` and Task 4's six tests.

Run: `cd /mnt/f/Work/Playground/Concat && python3 scripts/locales.py --check`
Expected: exits 0 with nothing out of step. If it reports a key as unused, the key's spelling differs between `en.json` and the `t("…")`/`I18n.t("…")` call; fix the call.

Run: `wcargo clippy -p concat -- -D warnings`
Expected: no warnings.

- [ ] **Step 11: Try it by hand**

Run `concat-dev`, then in the app:
1. With nothing selected, hover the new tray button: it's greyed, and its label says to select a clip with sound.
2. Import a talking-head recording and place it. Select it and click Remove Silences: the sheet opens with −40 dB, 0.5 s and 0.1 s, and a summary line.
3. Drag Silence level: the summary's numbers change as you drag.
4. Click Apply: the clip becomes pieces with no gaps, and a toast says how many pauses were removed.
5. Press Ctrl+Z: the whole clip is back in one step.
6. Detach the clip's audio (right-click → detach), select both, and Apply again: picture and sound are cut identically.
7. Reopen the sheet: it shows the numbers you last used.

- [ ] **Step 12: Commit**

```bash
wgit add src/crates/concat/src src/crates/concat/ui src/crates/concat/locales
wgit commit -m "\"feat(window): Remove Silences, a tray tool and sheet over the selected clips\"" -m "\"Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>\""
```

---

### Task 6: Shading on the lanes, and landing the spec and plan

**Files:**
- Modify: `src/crates/concat/ui/timeline/lanes.slint` (the model import at line 8, a property near `in property <DropData> drop;` at line 1131, and the overlay after the drop ghost around line 1494)
- Modify: `src/crates/concat/ui/workspace/timeline-pane.slint` (pass `silence-shades` to the lanes next to `drop: root.drop;` at line 431)
- Modify: `CHANGELOG.md` (one line under the newest unreleased heading)
- Commit: `docs/superpowers/specs/2026-09-29-silence-remover-design.md` and `docs/superpowers/plans/2026-09-29-silence-remover.md`

**Interfaces:**
- Consumes: `SilenceShadeData { row, start, length }` (Task 5); lanes' `root.at(seconds)`, `root.row-y(row)`, `root.tracks[row].height` and `root.px-per-second`.
- Produces: nothing.

- [ ] **Step 1: The lanes take the shades**

In `lanes.slint`, add `SilenceShadeData` to `import { ClipData, ClipKind, DropData, TimelineTool, TrackData } from "model.slint";`. Next to `in property <DropData> drop;`, add:
```slint
    /// The spans the open Remove Silences sheet would take out, shaded over
    /// their clips. Empty while the sheet is closed.
    in property <[SilenceShadeData]> silence-shades;
```
After the ghost's `if root.drop.active && landing.has-drag: Rectangle { ... }` block ends, add:
```slint
        // ── the silences the open sheet would take out ───────────────────
        //
        // Over the clips and under nothing else. Alpha in the paint, not
        // `opacity`, for the reason the ghost gives: a long span would
        // otherwise ask the GPU for a texture as wide as itself.
        for shade in root.silence-shades: Rectangle {
            x: root.at(shade.start);
            y: root.row-y(shade.row) + 4px;
            width: Math.max(1px, shade.length * root.px-per-second);
            height: Math.max(2px, root.tracks[shade.row].height - 10px);
            border-radius: 2px;
            background: Theme.danger.with-alpha(0.35);
            border-width: 1px;
            border-color: Theme.danger.with-alpha(0.7);
        }
```
In `timeline-pane.slint`, on the lanes element next to `drop: root.drop;`, add `silence-shades: root.silence-shades;`.

- [ ] **Step 2: Build and look**

Run: `wcargo build --profile quick -p concat --bin concat`
Expected: builds.

Run `concat-dev`, select a recording and open Remove Silences. Red shaded spans cover the pauses on the clip, and on its detached sound if it has one. They move as Silence level is dragged, and vanish on Cancel and on Apply. Scroll and zoom the timeline: the shades stay on their pauses.

- [ ] **Step 3: The changelog**

Add one line under the newest unreleased heading of `CHANGELOG.md`, in that section's style:
```markdown
- Remove Silences: select clips, open it from the timeline tray, and the pauses below a level you set are cut out and the gaps closed, in one undo step.
```

- [ ] **Step 4: The whole suite**

Run: `wcargo test -p concat-media -p concat-project -p concat -p concat-ui`
Run: `wcargo test -p concat-host --test export`
Expected: PASS, both.

- [ ] **Step 5: Commit, landing the spec and plan**

```bash
wgit add src/crates/concat/ui CHANGELOG.md docs/superpowers/specs/2026-09-29-silence-remover-design.md docs/superpowers/plans/2026-09-29-silence-remover.md
wgit commit -m "\"feat(timeline): the open Remove Silences sheet shades what it would cut\"" -m "\"Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>\""
```
