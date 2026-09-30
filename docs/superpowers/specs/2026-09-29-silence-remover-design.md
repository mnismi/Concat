# Remove Silences: design

Date: 2026-09-29 · Branch: `feat/silence-remover`

## Goal

Select a clip on the timeline, open **Remove Silences**, set how quiet counts
as silence, press Apply: the clip is cut into its spoken parts, the pauses are
gone, and the pieces close up with no gaps. Like DaVinci Resolve's silence
detection, and meant first for talking-head footage (tutorials, vlogs, podcast
clips).

What the user asked for:

- Works on the **selected clips** only (one or more). Other clips and tracks
  are left alone.
- A **dialog with settings**, a live preview of what will be removed, then
  **Apply**.
- **Silence only** in this version. Filler words ("um", "uh") through Whisper
  are a later feature on top of the same dialog.

## The dialog

Opened from a **Remove Silences** button in the timeline tray
(`ui/timeline/tray.slint`), beside captions and detach.

| Setting | Meaning | Range | Default |
|---|---|---|---|
| Silence level | Quieter than this counts as silence | −60 to −20 dB | −40 dB |
| Minimum pause | Only pauses longer than this are cut | 0.1 to 3 s | 0.5 s |
| Padding | Sound kept on each side of speech | 0 to 0.5 s | 0.1 s |

- **Live summary**, recomputed on every slider move:
  "Finds 14 pauses · removes 1 m 32 s · 6 m 10 s → 4 m 38 s", summed over
  every selected clip.
- **Timeline preview**: while the dialog is open, the spans that will be
  removed are shaded on each selected clip (`ui/timeline/lanes.slint`).
- **Cancel** closes with no change. **Apply** cuts, then closes.
- The three settings are **remembered** in the window's preferences
  (`concat/src/prefs.rs`) and reopen as last used.

Refusals, shown in place rather than as errors:

- **Button disabled** when nothing in the selection can be processed. The
  tooltip names the reason: no clip selected, the clip has no sound, or the
  clip has a speed curve.
- In a mixed selection, the clips that cannot be processed are skipped, and
  the summary says how many ("2 clips skipped: no sound").
- A clip whose waveform is still being read (just imported) counts as
  skipped, with the reason "the sound is still being read", and the summary
  updates when the waveform arrives.
- **Apply disabled**, with the reason in the summary line, when no pauses are
  found ("No pauses found at this level"), or when a clip would lose all its
  sound ("The whole clip is below this level").

## Detection (`concat-media/src/silence.rs`)

A pure function with no files and no UI:

```rust
pub struct SilenceSettings { pub level_db: f32, pub min_pause: f64, pub padding: f64 }

/// Spans of the source to remove, in source seconds, sorted and disjoint.
pub fn find_silences(peaks: &Peaks, from: f64, to: f64, settings: &SilenceSettings)
    -> Vec<(f64, f64)>;
```

- **Input**: the waveform peaks the timeline already holds for the clip's
  media and stream (`concat_host::media::peaks`, 1000 buckets a second,
  cached in the project). No second decode, so the preview is instant.
- **Loudness curve**: each bucket's amplitude is `max(|min|, max)`. The curve
  is the loudest bucket within 5 ms either side of each point (a 10 ms hold,
  so a voice's zero crossings never read as quiet). Past the end of the
  audio is silence.
- **Loud and quiet**: a bucket is loud when the curve reaches the level
  (`10^(level_db / 20)` as an amplitude). A loud run shorter than 30 ms is
  a click, not speech, and counts as quiet, so it never breaks a pause.
- **Minimum, then padding**: a quiet run shorter than `min_pause` is kept
  (a pause between words). A longer one is shrunk by `padding` at each end,
  and dropped if less than 20 ms is left. A run that touches `from` or `to`
  is only shrunk at its inner end, so leading and trailing silence goes
  completely.

## The cut (`Command::RemoveClipRanges`)

A new command in `concat-project/src/commands/`, next to split and remove in
`clips.rs`:

```rust
RemoveClipRanges {
    /// The clip to cut.
    clip_id: String,
    /// Spans of its source to take out, in source seconds.
    ranges: Vec<(f64, f64)>,
}
```

- **Mapping**: a clip shows the source span from `source_start` to
  `source_start + duration × speed`. Source time `s` sits at timeline time
  `start + (s − source_start) / speed`. Ranges are clamped to the clip,
  sorted and merged first.
- **Cutting**: split at each range boundary, then remove the pieces inside
  the ranges, with ripple, using the existing split and ripple-remove code.
  So keyframes, fades and transitions behave exactly as with a manual cut:
  keys are re-anchored, the fade-in stays on the first kept piece, the
  fade-out on the last, and the in-transition on the first.
- **Gaps**: later clips on the same track move left by the removed length.
  **Other tracks do not move.** A title on another track keeps its timeline
  position.
- **Detached sound**: when the clip's audio was detached (a clip whose
  `detached_from` names this one), the same source ranges are cut from that
  sound clip through its own mapping, with ripple on its track. Picture and
  voice stay in sync, including when the sound was nudged. The link works in
  both directions: given the detached sound clip, the command cuts its video
  too. When a video and its detached sound are both selected, the window
  sends one command for the pair, so the pair is never cut twice.
- **Undo**: the window sends one `Batch` of one `RemoveClipRanges` per
  processed clip: one undo step for the whole Apply.
- **Refusals** (new `CommandError` variants): the clip is gone, it has a
  speed curve, or the ranges would remove the whole clip. Empty ranges are a
  no-op.
- **API**: nothing to add. The API and the command line pass `Command`
  through unchanged, so `RemoveClipRanges` is scriptable as soon as it
  exists.

## The window

- `concat/src/panes/silence.rs`: a pane in the shape of `captions.rs`,
  holding the dialog's state, `Msg`, `update` and `data`. It reads each
  selected clip's cached peaks from the media art the window already holds.
  It recomputes the ranges on every setting change and publishes them for
  the preview. On Apply it sends the `Batch`.
- `concat/ui/dialogs/silence.slint`: the sheet, built from the existing
  primitives (sliders, modal, buttons).
- **Strings**: new keys in `concat/locales/en.json` and in all 13 other
  shipped locales. The locale test requires full coverage, and the other
  languages are machine-translated for a native speaker to check.

## Testing

- **Detection** (unit, `silence.rs`), on synthetic peaks: speech with gaps; a
  pause shorter than the minimum; a click inside a pause; leading and
  trailing silence; all silence; no silence; padding larger than half a
  pause.
- **Command** (unit, `concat-project`): piece positions and lengths, a
  later clip on the same track moves left, a clip on another track does not
  move, detached sound is cut in sync, keys and fades land on the right
  pieces, a speed-2 clip maps correctly, the speed-curve and whole-clip
  refusals, and one undo restores the project exactly.
- **End to end** (`concat-host/tests/export.rs`): a synthetic clip of tone
  and silence goes through `RemoveClipRanges` and exports, and the file has
  the expected length.
- **By hand**: once in the app, on a real recording.

## Not in this version

- Filler-word removal (Whisper).
- Clips with a speed curve.
- Moving other tracks to stay in sync with the cut.
- Running over the whole timeline without a selection.
- The phone layout's tool bar. The desktop tray is the only way in.
