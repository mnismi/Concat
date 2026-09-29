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
