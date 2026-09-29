# Clip Animations (In and Out) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every video, still and title can have an In animation over its first seconds and an Out animation over its last, picked from motion and effect presets with a length, on a new Animations tab.

**Architecture:**
- **Model:** the project stores a preset id and a length per end (`Clip::animation_in`, `Clip::animation_out`), set by one new command.
- **Motion:** a pure table in `concat-core::motion` turns the id and the progress through its window into a relative `Motion`. The engine clip applies it on top of its keyframes, so preview and export draw the same frames.
- **Effect presets:** these add a spanned effect link with keyed strength when the clip is flattened. The link is never written into the project.

**Tech Stack:** Rust (workspace under `src/`), Slint UI, WGSL effect packages, serde JSON documents.

**Spec:** `docs/superpowers/specs/2026-09-29-clip-animations-design.md`

## Global Constraints

- **Kinds:** video, image and text can animate. Audio clips and effect layers are refused with `CommandError::CannotAnimate`.
- **Serde:** `animationIn` and `animationOut` are optional and skipped when empty, so a project without animations writes exactly what it wrote before.
- **Unknown ids:** a preset id this build does not know is kept in the file and drawn as no animation.
- **Length:** at least `MIN_ANIMATION` = 0.1 s and at most the clip's length. Setting one end so that In plus Out outlasts the clip shortens the other end, and the other end is dropped if less than 0.1 s would be left.
- **Splits:** the In stays on the first piece and the Out on the last, like `fade_in` and `fade_out`. This covers split, freeze-frame, merge and silence removal.
- **UI strings:** every new string goes in `src/crates/concat/locales/en.json` and all 13 other locales, where the locale test demands full coverage.
- **Commit trailer:** every commit message ends with `Co-Authored-By: Claude Opus 5.5 (1M context) <noreply@anthropic.com>`.
- **Don't commit this plan or the spec until the last task.** Both land in the commit that finishes Task 6 (user CLAUDE.md).
- **Build environment (this machine):** cargo runs on Windows from WSL through the scratchpad helpers `wcargo` and `wgit` (see memory `concat-dev-setup`). Commit with `wgit commit -F <file>`, never `-m`. The running `concat-dev` app locks `target\quick\concat.exe`, so verify with `wcargo test` and `wcargo clippy`, not a quick-profile build.

## Review Focus

1. **A clip trimmed shorter than In + Out:** both windows must still fit and never overlap. `fit_animations` shrinks them in proportion (Task 1 test `a_trim_shrinks_both_ends_to_fit`), and the flattener clamps again (Task 3 test `the_flattener_holds_the_windows_inside_the_clip`).
2. **A keyed clip that also animates:** the preset must ride on the keys, not replace them (Task 2 test `motion_rides_on_the_keys`).
3. **An animated title:** titles flatten through `concat-host/src/titles.rs`, not `flatten.rs`, and forgetting that path leaves titles still (Task 3 step 5, Task 6 step 7).
4. **Pop starting at scale 0:** the engine must never divide by or draw a zero scale (Task 2 test `pop_starts_invisible_but_never_at_zero_scale`).
5. **An effect preset on a sped-up clip:** the span is in source seconds, so the window must follow the speed (Task 6 test `an_effect_window_follows_the_speed`).

---

### Task 1: The model and the command

**Files:**
- Modify: `src/crates/concat-project/src/model.rs` (add `ClipAnimation` after `struct Transition` (~line 840); `Clip` fields after `transition_in` (~line 1242); `Clip::blank` (~line 1335); a new `impl Clip` block)
- Modify: `src/crates/concat-project/src/commands/mod.rs` (the model import at line 15; `AnimationSlot`; `Command::SetClipAnimation`; `CommandError::CannotAnimate`; `has_non_finite`; dispatch at ~line 1065)
- Modify: `src/crates/concat-project/src/commands/properties.rs` (the new arm; `fit_animations` in `SetClipSpeed` and `SetClipSpeedCurve`)
- Modify: `src/crates/concat-project/src/commands/clips.rs` (TrimClip, SplitClips, FreezeFrame, MergeClips)
- Modify: `src/crates/concat-project/src/commands/cut.rs` (`keep_the_ends`)
- Test: `src/crates/concat-project/src/lib.rs` (the `mod tests`)

**Interfaces:**
- Produces:
  - `concat_project::model::ClipAnimation { pub id: String, pub duration: f64 }`
  - `concat_project::model::MIN_ANIMATION: f64 = 0.1`
  - `Clip::animation_in: Option<ClipAnimation>` and `Clip::animation_out: Option<ClipAnimation>`
  - `Clip::can_animate(&self) -> bool` and `Clip::fit_animations(&mut self)`
  - `concat_project::commands::AnimationSlot { In, Out }`, which serializes as `"in"` and `"out"`
  - `Command::SetClipAnimation { clip_id: String, slot: AnimationSlot, animation: Option<ClipAnimation> }`, which serializes as `{"op":"setClipAnimation","clipId":…,"slot":"in","animation":{"id":…,"duration":…}}`
  - `CommandError::CannotAnimate`

- [ ] **Step 1: Write the failing tests**

Add to the `use` block at the top of `mod tests` in `src/crates/concat-project/src/lib.rs`:

```rust
    use crate::commands::AnimationSlot;
    use crate::model::{Clip, ClipAnimation};
```

Add these tests and helpers at the end of `mod tests` (after `a_cut_group_is_the_pieces_that_play_together` and its neighbours):

```rust
    fn animation(id: &str, duration: f64) -> ClipAnimation {
        ClipAnimation {
            id: id.to_owned(),
            duration,
        }
    }

    fn animate(editor: &mut Editor, clip: &str, slot: AnimationSlot, id: &str, duration: f64) {
        editor
            .apply(Command::SetClipAnimation {
                clip_id: clip.to_owned(),
                slot,
                animation: Some(animation(id, duration)),
            })
            .expect("animates");
    }

    fn clip_of(editor: &Editor, id: &str) -> Clip {
        editor.project().active().clip(id).expect("there").clone()
    }

    #[test]
    fn an_animation_is_set_replaced_and_removed() {
        let (mut editor, _, clip) = fixture();
        animate(&mut editor, &clip, AnimationSlot::In, "zoom-in", 1.0);
        assert_eq!(clip_of(&editor, &clip).animation_in, Some(animation("zoom-in", 1.0)));
        animate(&mut editor, &clip, AnimationSlot::In, "fade-in", 0.5);
        assert_eq!(clip_of(&editor, &clip).animation_in, Some(animation("fade-in", 0.5)));
        assert_eq!(clip_of(&editor, &clip).animation_out, None);
        editor
            .apply(Command::SetClipAnimation {
                clip_id: clip.clone(),
                slot: AnimationSlot::In,
                animation: None,
            })
            .expect("removes");
        assert_eq!(clip_of(&editor, &clip).animation_in, None);
    }

    #[test]
    fn an_animation_is_held_to_the_clip() {
        let (mut editor, _, clip) = fixture();
        animate(&mut editor, &clip, AnimationSlot::Out, "fade-out", 25.0);
        assert_eq!(clip_of(&editor, &clip).animation_out, Some(animation("fade-out", 10.0)));
        animate(&mut editor, &clip, AnimationSlot::Out, "fade-out", 0.01);
        assert_eq!(clip_of(&editor, &clip).animation_out, Some(animation("fade-out", 0.1)));
    }

    #[test]
    fn the_other_end_gives_way() {
        let (mut editor, _, clip) = fixture();
        animate(&mut editor, &clip, AnimationSlot::In, "zoom-in", 6.0);
        animate(&mut editor, &clip, AnimationSlot::Out, "zoom-out", 6.0);
        let placed = clip_of(&editor, &clip);
        assert_eq!(placed.animation_out, Some(animation("zoom-out", 6.0)));
        assert_eq!(placed.animation_in, Some(animation("zoom-in", 4.0)));
        // An end that takes the whole clip leaves no room: the other goes.
        animate(&mut editor, &clip, AnimationSlot::In, "zoom-in", 10.0);
        assert_eq!(clip_of(&editor, &clip).animation_out, None);
    }

    #[test]
    fn a_sound_or_a_layer_cannot_animate() {
        let mut editor = Editor::new();
        let Command::AddMedia { mut item } = media("/a.wav", 5.0, true) else {
            unreachable!()
        };
        item.kind = MediaKind::Audio;
        let media_id = editor
            .apply(Command::AddMedia { item })
            .expect("adds")
            .created_id
            .expect("id");
        let track_id = editor.project().active().tracks[0].id.clone();
        let sound = editor
            .apply(Command::AddClip {
                media_id,
                track_id,
                start: 0.0,
                ripple: false,
            })
            .expect("adds")
            .created_id
            .expect("id");
        let layer = editor
            .apply(Command::AddLayerClip {
                track_id: None,
                start: 20.0,
                duration: Some(2.0),
                effect_id: "concat.warm".to_owned(),
                name: "Warm".to_owned(),
            })
            .expect("adds")
            .created_id
            .expect("id");
        for clip in [sound, layer] {
            assert_eq!(
                editor.apply(Command::SetClipAnimation {
                    clip_id: clip,
                    slot: AnimationSlot::In,
                    animation: Some(animation("fade-in", 1.0)),
                }),
                Err(CommandError::CannotAnimate)
            );
        }
        assert_eq!(
            editor.apply(Command::SetClipAnimation {
                clip_id: "nowhere".to_owned(),
                slot: AnimationSlot::In,
                animation: None,
            }),
            Err(CommandError::ClipGone)
        );
    }

    #[test]
    fn a_title_can_animate() {
        let mut editor = Editor::new();
        let title = editor
            .apply(Command::AddTextClip {
                above: false,
                track_id: None,
                start: 0.0,
                style: None,
                duration: Some(3.0),
                offset_y: None,
            })
            .expect("adds")
            .created_id
            .expect("id");
        animate(&mut editor, &title, AnimationSlot::In, "pop", 0.5);
        assert_eq!(clip_of(&editor, &title).animation_in, Some(animation("pop", 0.5)));
    }

    #[test]
    fn a_split_keeps_in_on_the_head_and_out_on_the_tail() {
        let (mut editor, _, clip) = fixture();
        animate(&mut editor, &clip, AnimationSlot::In, "zoom-in", 1.0);
        animate(&mut editor, &clip, AnimationSlot::Out, "zoom-out", 1.0);
        editor
            .apply(Command::SplitClips {
                clip_ids: vec![clip.clone()],
                time: 5.0,
            })
            .expect("splits");
        let mut clips = editor.project().active().clips.to_vec();
        clips.sort_by(|a, b| a.start.total_cmp(&b.start));
        assert_eq!(clips[0].animation_in, Some(animation("zoom-in", 1.0)));
        assert_eq!(clips[0].animation_out, None);
        assert_eq!(clips[1].animation_in, None);
        assert_eq!(clips[1].animation_out, Some(animation("zoom-out", 1.0)));
    }

    #[test]
    fn a_merge_keeps_the_first_in_and_the_last_out() {
        let (mut editor, _, clip) = fixture();
        animate(&mut editor, &clip, AnimationSlot::In, "zoom-in", 1.0);
        animate(&mut editor, &clip, AnimationSlot::Out, "zoom-out", 1.0);
        let tail = editor
            .apply(Command::SplitClips {
                clip_ids: vec![clip.clone()],
                time: 5.0,
            })
            .expect("splits")
            .created_id
            .expect("the tail");
        editor
            .apply(Command::MergeClips {
                clip_ids: vec![clip.clone(), tail],
            })
            .expect("merges");
        let merged = clip_of(&editor, &clip);
        assert_eq!(merged.animation_in, Some(animation("zoom-in", 1.0)));
        assert_eq!(merged.animation_out, Some(animation("zoom-out", 1.0)));
    }

    #[test]
    fn removing_silences_keeps_the_ends_animated() {
        let (mut editor, _, clip) = fixture();
        animate(&mut editor, &clip, AnimationSlot::In, "zoom-in", 0.5);
        animate(&mut editor, &clip, AnimationSlot::Out, "zoom-out", 0.5);
        editor
            .apply(Command::RemoveClipRanges {
                clip_id: clip,
                ranges: vec![(0.0, 1.0), (4.0, 5.0), (9.0, 10.0)],
            })
            .expect("cuts");
        let mut clips = editor.project().active().clips.to_vec();
        clips.sort_by(|a, b| a.start.total_cmp(&b.start));
        assert_eq!(clips.len(), 2);
        assert_eq!(clips[0].animation_in, Some(animation("zoom-in", 0.5)));
        assert_eq!(clips[0].animation_out, None);
        assert_eq!(clips[1].animation_in, None);
        assert_eq!(clips[1].animation_out, Some(animation("zoom-out", 0.5)));
    }

    #[test]
    fn a_trim_shrinks_both_ends_to_fit() {
        let (mut editor, _, clip) = fixture();
        animate(&mut editor, &clip, AnimationSlot::In, "zoom-in", 4.0);
        animate(&mut editor, &clip, AnimationSlot::Out, "zoom-out", 4.0);
        editor
            .apply(Command::TrimClip {
                clip_id: clip.clone(),
                edge: TrimEdge::End,
                delta: -8.0,
                ripple: false,
            })
            .expect("trims");
        let trimmed = clip_of(&editor, &clip);
        assert!((trimmed.duration - 2.0).abs() < 1e-9);
        let (a, b) = (
            trimmed.animation_in.expect("kept").duration,
            trimmed.animation_out.expect("kept").duration,
        );
        assert!((a - 1.0).abs() < 1e-9 && (b - 1.0).abs() < 1e-9, "{a} {b}");
    }

    #[test]
    fn one_undo_takes_an_animation_away() {
        let (mut editor, _, clip) = fixture();
        let before = editor.project().clone();
        animate(&mut editor, &clip, AnimationSlot::In, "zoom-in", 1.0);
        assert!(editor.undo());
        assert_eq!(*editor.project(), before);
    }

    #[test]
    fn a_project_without_animations_writes_no_animation_fields() {
        let (editor, _, _) = fixture();
        let document = crate::to_document(&settings(), editor.project()).to_string();
        assert!(!document.contains("animationIn"), "{document}");
        assert!(!document.contains("animationOut"), "{document}");
    }

    #[test]
    fn an_unknown_preset_round_trips() {
        let (mut editor, _, clip) = fixture();
        animate(&mut editor, &clip, AnimationSlot::Out, "from-a-newer-build", 0.7);
        let document = crate::to_document(&settings(), editor.project());
        assert!(document.to_string().contains("\"animationOut\""));
        let loaded = Editor::from_document(&document).expect("loads");
        assert_eq!(
            clip_of(&loaded, &clip).animation_out,
            Some(animation("from-a-newer-build", 0.7))
        );
    }
```

If `Project` does not implement `PartialEq`, compare `to_document(&settings(), …)` of the two projects instead in `one_undo_takes_an_animation_away`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `wcargo test -p concat-project --lib 2>&1 | tail -20`
Expected: a compile error, because `AnimationSlot`, `ClipAnimation`, `Command::SetClipAnimation` and `CommandError::CannotAnimate` don't exist yet.

- [ ] **Step 3: Add the model**

In `src/crates/concat-project/src/model.rs`, directly after the `Transition` struct:

```rust
/// A preset played over one end of a clip: its In over the first seconds,
/// its Out over the last. What each id draws is `concat_core::motion`'s;
/// an id this build does not know is kept, and drawn as no animation.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipAnimation {
    /// Which preset, e.g. "zoom-in". Stable forever: project files store it.
    pub id: String,
    /// Seconds of timeline it covers, from the end of the clip it sits on.
    #[serde(default = "half")]
    pub duration: f64,
}

/// The shortest animation a command keeps, in seconds: three frames at
/// thirty, about the least that still reads as movement.
pub const MIN_ANIMATION: f64 = 0.1;

fn half() -> f64 {
    0.5
}
```

In `pub struct Clip`, directly after the `transition_in` field:

```rust
    /// The animation over the clip's first seconds, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "wire::maybe")]
    pub animation_in: Option<ClipAnimation>,
    /// The animation over its last seconds, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(deserialize_with = "wire::maybe")]
    pub animation_out: Option<ClipAnimation>,
```

In `Clip::blank`, after `transition_in: None,`:

```rust
            animation_in: None,
            animation_out: None,
```

Add a new `impl Clip` block just before `impl Default for Clip`:

```rust
impl Clip {
    /// Whether this clip has a picture to move: a video, a still or a
    /// title. A sound has nothing to show and a layer is a treatment of
    /// what is beneath it, so neither takes an In or an Out.
    pub fn can_animate(&self) -> bool {
        matches!(self.kind, ClipKind::Video | ClipKind::Image | ClipKind::Text)
    }

    /// Holds both animations inside the clip: when the two together
    /// outlast it they shrink in proportion, so a trimmed clip keeps both
    /// ends' character rather than losing one. What every edit that
    /// shortens a clip runs.
    pub fn fit_animations(&mut self) {
        let length = self.duration;
        let taken = self.animation_in.as_ref().map_or(0.0, |a| a.duration)
            + self.animation_out.as_ref().map_or(0.0, |a| a.duration);
        if taken <= length || taken <= 0.0 {
            return;
        }
        let scale = length / taken;
        for animation in [&mut self.animation_in, &mut self.animation_out]
            .into_iter()
            .flatten()
        {
            animation.duration *= scale;
        }
    }
}
```

- [ ] **Step 4: Add the command**

In `src/crates/concat-project/src/commands/mod.rs`, add `ClipAnimation` and `MIN_ANIMATION` to the `use crate::model::{…}` list at line 15.

Before `pub enum Command`, directly after the `double_option` module, add:

```rust
/// Which end of a clip an animation plays at.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnimationSlot {
    /// Over the clip's first seconds, into its resting state.
    In,
    /// Over its last seconds, out of it.
    Out,
}
```

In `pub enum Command`, directly after the `RemoveClipRanges` variant:

```rust
    /// Sets the animation at one end of a clip, or takes it away. The
    /// length is held to [`MIN_ANIMATION`] and the clip's length; when the
    /// two ends together would outlast the clip, the other end is shortened
    /// to fit, and dropped when less than [`MIN_ANIMATION`] would be left -
    /// the end being set keeps what was asked. Refused for a clip that is
    /// not there and for a kind with no picture to move.
    SetClipAnimation {
        /// The clip to animate.
        clip_id: String,
        /// Which end.
        slot: AnimationSlot,
        /// The preset and its length; None removes it.
        animation: Option<ClipAnimation>,
    },
```

In `pub enum CommandError`, change the doc comment of `ClipGone` to `/// A clip command named a clip that is not there.`, and add after `NothingLeft`:

```rust
    /// [`Command::SetClipAnimation`] on a sound or a layer.
    #[error("Only a video, a still or a title can be animated.")]
    CannotAnimate,
```

In `has_non_finite`, after the `Command::RemoveClipRanges` arm:

```rust
            Command::SetClipAnimation { animation, .. } => {
                bad(animation.iter().map(|animation| animation.duration))
            }
```

In the dispatch, add `| Command::SetClipAnimation { .. }` to the group that begins `command @ (Command::UpdateClip { .. }`, so it goes to `properties::apply`.

- [ ] **Step 5: Apply it in properties.rs**

In `src/crates/concat-project/src/commands/properties.rs`, add a new arm next to `Command::SetClipSpeed`:

```rust
        Command::SetClipAnimation {
            clip_id,
            slot,
            animation,
        } => {
            let clip = project
                .active_mut()
                .clip_mut(&clip_id)
                .ok_or(CommandError::ClipGone)?;
            if !clip.can_animate() {
                return Err(CommandError::CannotAnimate);
            }
            let length = clip.duration;
            let animation = animation.map(|animation| ClipAnimation {
                duration: animation.duration.clamp(MIN_ANIMATION.min(length), length),
                ..animation
            });
            let room = length - animation.as_ref().map_or(0.0, |a| a.duration);
            let (mine, other) = match slot {
                AnimationSlot::In => (&mut clip.animation_in, &mut clip.animation_out),
                AnimationSlot::Out => (&mut clip.animation_out, &mut clip.animation_in),
            };
            let mut applied = assign(mine, animation);
            if other.as_ref().is_some_and(|other| other.duration > room) {
                applied = true;
                if room < MIN_ANIMATION {
                    *other = None;
                } else if let Some(other) = other {
                    other.duration = room;
                }
            }
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }
```

In `Command::SetClipSpeed` and `Command::SetClipSpeedCurve`, directly before each `Ok(Outcome {`, add `clip.fit_animations();`.

- [ ] **Step 6: Carry the animations through splits, freezes, merges, trims and cuts**

In `src/crates/concat-project/src/commands/clips.rs`:

- **SplitClips** (~line 288): after `tail.fade_in = 0.0;` add `tail.animation_in = None;`. After `head.fade_out = 0.0;` (~line 295) add `head.animation_out = None;`.
- **FreezeFrame**:
  - after `tail.fade_in = 0.0;` (~line 448) add `tail.animation_in = None;`;
  - after `head.fade_out = 0.0;` (~line 453) add `head.animation_out = None;`;
  - after `frozen.transition_in = None;` (~line 485) add:
    ```rust
            frozen.animation_in = None;
            frozen.animation_out = None;
    ```
- **MergeClips**: after `survivor.fade_out = last.fade_out;` (~line 524) add `survivor.animation_out = last.animation_out.clone();`.
- **TrimClip**: in both edges, directly after each `clip.rewindow_keys(…);` inside `if applied { … }`, add `clip.fit_animations();`.

In `src/crates/concat-project/src/commands/cut.rs`, change `keep_the_ends` so that its two `if let` blocks read:

```rust
    if let Some(clip) = timeline.clip_mut(&first) {
        clip.fade_in = whole.fade_in.min(clip.duration);
        clip.transition_in = whole.transition_in.clone();
        clip.animation_in = whole.animation_in.clone();
        clip.fit_animations();
    }
    if let Some(clip) = timeline.clip_mut(&last) {
        clip.fade_out = whole.fade_out.min(clip.duration);
        clip.animation_out = whole.animation_out.clone();
        clip.fit_animations();
    }
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `wcargo test -p concat-project --lib 2>&1 | tail -20`
Expected: every test passes (121 before this task plus the 12 new ones).

Run: `wcargo clippy -p concat-project --all-targets -- -D warnings 2>&1 | tail -5`
Expected: no warnings.

Run: `wcargo fmt --all --check && echo FMT_CLEAN`
Expected: `FMT_CLEAN`. If it fails, run `wcargo fmt --all`.

- [ ] **Step 8: Commit**

```bash
wgit add src/crates/concat-project
# msg.txt: "feat(project): an In and an Out animation on each clip" + trailer
wgit commit -F .superpowers/sdd/2026-09-29-clip-animations/msg.txt
```

---

### Task 2: Motion presets in the engine

**Files:**
- Create: `src/crates/concat-core/src/motion.rs`
- Modify: `src/crates/concat-core/src/lib.rs` (add `pub mod motion;` after `pub mod frame;`)
- Modify: `src/crates/concat-core/src/timeline.rs` (the `Clip` fields at ~line 94; `Clip::new` ~line 216; `transform_at` and `opacity_at` ~lines 232-246; tests)

**Interfaces:**
- Consumes: nothing from Task 1. `concat-core` depends on no other crate.
- Produces:
  - `concat_core::motion::Motion { scale, offset_x, offset_y, rotation, opacity: f64 }`, with `Motion::IDENTITY` and `Motion::then(self, other) -> Motion`
  - `concat_core::motion::{Side, Group, EffectRamp, Preset, PRESETS, preset(id) -> Option<&'static Preset>, motion_at(id, progress) -> Motion, Played}`
  - `Preset { id: &'static str, side: Side, group: Group, label: &'static str, effect: Option<EffectRamp> }`, where `label` is the full i18n key
  - `EffectRamp { effect: &'static str, param: &'static str, from: f64, rest: f64 }`
  - `Played { id: String, seconds: f64 }`
  - on `concat_core::timeline::Clip`: `pub entrance: Option<Played>`, `pub exit: Option<Played>` and `pub fn motion_at(&self, time: Rational) -> Motion`

- [ ] **Step 1: Write the failing tests**

Create `src/crates/concat-core/src/motion.rs` holding only its tests, so they fail to compile:

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

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
            assert!(near(rest.offset_x, 0.0) && near(rest.offset_y, 0.0), "{}", preset.id);
            assert!(near(rest.rotation, 0.0) && near(rest.opacity, 1.0), "{}", preset.id);
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
                assert_eq!(motion_at(way_in, p), motion_at(way_out, p), "{way_out} at {p}");
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
                PRESETS[index + 1..].iter().all(|other| other.id != preset.id),
                "{} twice",
                preset.id
            );
            assert!(preset.label.starts_with("animations.preset."), "{}", preset.id);
        }
    }
}
```

In `src/crates/concat-core/src/lib.rs`, add `pub mod motion;` after `pub mod frame;`.

Add to `mod tests` in `src/crates/concat-core/src/timeline.rs`:

```rust
    use crate::motion::Played;

    fn animated(entrance: Option<(&str, f64)>, exit: Option<(&str, f64)>) -> Clip {
        let mut clip = Clip::new(MediaRef::new("a.mp4"), seconds(2), seconds(10));
        let played = |(id, seconds): (&str, f64)| Played {
            id: id.to_owned(),
            seconds,
        };
        clip.entrance = entrance.map(played);
        clip.exit = exit.map(played);
        clip
    }

    #[test]
    fn a_clip_zooms_in_over_its_entrance_and_rests_after() {
        let clip = animated(Some(("zoom-in", 1.0)), None);
        assert!((clip.transform_at(seconds(2)).scale - 0.6).abs() < 1e-9);
        assert_eq!(clip.opacity_at(seconds(2)), 0.0);
        assert_eq!(clip.transform_at(seconds(5)), clip.transform);
        assert_eq!(clip.opacity_at(seconds(5)), 1.0);
    }

    #[test]
    fn a_clip_fades_out_over_its_exit() {
        let clip = animated(None, Some(("fade-out", 2.0)));
        // One second before the end is halfway through a two-second exit.
        assert!((clip.opacity_at(seconds(11)) - 0.5).abs() < 1e-6);
        assert_eq!(clip.opacity_at(seconds(9)), 1.0);
    }

    #[test]
    fn motion_rides_on_the_keys() {
        use crate::animate::{Animation, Ease, Key, Track};
        let mut clip = animated(Some(("zoom-in", 1.0)), None);
        clip.animation = Some(Animation {
            scale: Track::new(vec![Key {
                at: 0.0,
                value: 2.0,
                ease: Ease::LINEAR,
            }]),
            ..Animation::default()
        });
        // Keyed to twice the size, and zoomed in from 0.6 of that.
        assert!((clip.transform_at(seconds(2)).scale - 1.2).abs() < 1e-9);
        assert!((clip.transform_at(seconds(6)).scale - 2.0).abs() < 1e-9);
    }

    #[test]
    fn pop_starts_invisible_but_never_at_zero_scale() {
        let clip = animated(Some(("pop", 0.5)), None);
        assert!(clip.transform_at(seconds(2)).scale > 0.0);
        assert_eq!(clip.opacity_at(seconds(2)), 0.0);
    }

    #[test]
    fn an_empty_window_plays_nothing() {
        let clip = animated(Some(("zoom-in", 0.0)), Some(("zoom-out", 0.0)));
        assert_eq!(clip.transform_at(seconds(2)), clip.transform);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `wcargo test -p concat-core --lib 2>&1 | tail -20`
Expected: a compile error, because `PRESETS`, `motion_at`, `preset`, `Motion`, `Side`, `Played` and `Clip::entrance` are not found.

- [ ] **Step 3: Write `motion.rs`**

Put this above the `#[cfg(test)]` block in `src/crates/concat-core/src/motion.rs`:

```rust
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
    Still,
    /// Opacity alone, eased at both ends.
    Fade,
    /// Opacity alone, reaching full at the halfway point.
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
    basic("zoom-in", Side::In, "animations.preset.zoomIn", Curve::Zoom(0.6)),
    basic("zoom-from-big", Side::In, "animations.preset.zoomFromBig", Curve::Zoom(1.6)),
    basic("slide-from-left", Side::In, "animations.preset.slideFromLeft", Curve::Slide(-0.3, 0.0)),
    basic("slide-from-right", Side::In, "animations.preset.slideFromRight", Curve::Slide(0.3, 0.0)),
    basic("slide-from-top", Side::In, "animations.preset.slideFromTop", Curve::Slide(0.0, -0.3)),
    basic("slide-from-bottom", Side::In, "animations.preset.slideFromBottom", Curve::Slide(0.0, 0.3)),
    basic("rise", Side::In, "animations.preset.rise", Curve::Slide(0.0, 0.08)),
    basic("spin-in", Side::In, "animations.preset.spinIn", Curve::Spin),
    basic("pop", Side::In, "animations.preset.pop", Curve::Pop),
    basic("shake-in", Side::In, "animations.preset.shakeIn", Curve::Shake),
    basic("fade-out", Side::Out, "animations.preset.fadeOut", Curve::Fade),
    basic("zoom-out", Side::Out, "animations.preset.zoomOut", Curve::Zoom(0.6)),
    basic("zoom-to-big", Side::Out, "animations.preset.zoomToBig", Curve::Zoom(1.6)),
    basic("slide-to-left", Side::Out, "animations.preset.slideToLeft", Curve::Slide(-0.3, 0.0)),
    basic("slide-to-right", Side::Out, "animations.preset.slideToRight", Curve::Slide(0.3, 0.0)),
    basic("slide-to-top", Side::Out, "animations.preset.slideToTop", Curve::Slide(0.0, -0.3)),
    basic("slide-to-bottom", Side::Out, "animations.preset.slideToBottom", Curve::Slide(0.0, 0.3)),
    basic("sink", Side::Out, "animations.preset.sink", Curve::Slide(0.0, 0.08)),
    basic("spin-out", Side::Out, "animations.preset.spinOut", Curve::Spin),
    basic("pop-out", Side::Out, "animations.preset.popOut", Curve::Pop),
    basic("shake-out", Side::Out, "animations.preset.shakeOut", Curve::Shake),
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
```

`Curve::Still` and `Curve::FadeFast` are unused until Task 6. If clippy flags them as dead code, put `#[allow(dead_code)] // the Effects presets of Task 6` on those two variants only, and remove it in Task 6.

- [ ] **Step 4: Put it on the engine clip**

In `src/crates/concat-core/src/timeline.rs`, add `use crate::motion::{self, Motion, Played};` beside `use crate::animate::Animation;`. After the `animation` field of `Clip`:

```rust
    /// The preset over the clip's first seconds, when it has one. See
    /// [`motion`].
    pub entrance: Option<Played>,
    /// The preset over its last seconds.
    pub exit: Option<Played>,
```

In `Clip::new`, after `animation: None,`, add `entrance: None,` and `exit: None,`.

Replace `transform_at` and `opacity_at` with:

```rust
    /// The placement at `time`: the clip's own, moved by its keys and then
    /// by its In and Out.
    pub fn transform_at(&self, time: Rational) -> Transform {
        let keyed = match &self.animation {
            Some(animation) => animation.transform_at(self.transform, self.fraction_at(time)),
            None => self.transform,
        };
        if self.entrance.is_none() && self.exit.is_none() {
            return keyed;
        }
        let motion = self.motion_at(time);
        Transform {
            scale: (keyed.scale * motion.scale).max(0.001),
            offset_x: keyed.offset_x + motion.offset_x,
            offset_y: keyed.offset_y + motion.offset_y,
            rotation: keyed.rotation + motion.rotation,
            ..keyed
        }
    }

    /// The opacity at `time`, before the fade ramps: the clip's own, scaled
    /// by its keys and then by its In and Out.
    pub fn opacity_at(&self, time: Rational) -> f32 {
        let keyed = match &self.animation {
            Some(animation) => animation.opacity_at(self.opacity, self.fraction_at(time)),
            None => self.opacity,
        };
        if self.entrance.is_none() && self.exit.is_none() {
            return keyed;
        }
        (f64::from(keyed) * self.motion_at(time).opacity).clamp(0.0, 1.0) as f32
    }

    /// What the In and Out do at `time`: the identity outside both
    /// windows, and both combined where a short clip's windows meet.
    pub fn motion_at(&self, time: Rational) -> Motion {
        let into = (time - self.start).as_f64();
        let left = (self.start + self.duration - time).as_f64();
        let mut out = Motion::IDENTITY;
        if let Some(played) = &self.entrance
            && played.seconds > 0.0
            && into < played.seconds
        {
            out = out.then(motion::motion_at(&played.id, into / played.seconds));
        }
        if let Some(played) = &self.exit
            && played.seconds > 0.0
            && left < played.seconds
        {
            out = out.then(motion::motion_at(&played.id, left / played.seconds));
        }
        out
    }
```

Every other `concat_core::timeline::Clip { … }` struct literal in the workspace also needs the two fields. Run `grep -rn "retime: None" src/crates --include=*.rs` and add `entrance: None, exit: None,` wherever a literal lists `animation:`. Most sites use `Clip::new`, which needs nothing.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `wcargo test -p concat-core --lib 2>&1 | tail -20`
Expected: every test passes, including the 7 in `motion` and the 5 new ones in `timeline`.

Run: `wcargo clippy -p concat-core --all-targets -- -D warnings 2>&1 | tail -5`
Expected: no warnings.

Run: `wcargo fmt --all`, then `wcargo fmt --all --check && echo FMT_CLEAN`
Expected: `FMT_CLEAN`.

- [ ] **Step 6: Commit**

```bash
wgit add src/crates/concat-core
# msg.txt: "feat(core): motion presets for a clip's In and Out" + trailer
wgit commit -F .superpowers/sdd/2026-09-29-clip-animations/msg.txt
```

---

### Task 3: The export and the preview draw them

**Files:**
- Modify: `src/crates/concat-export/src/lib.rs` (`ExportAnimation` next to `TransitionSpec`; `ExportClip` fields after `animation` ~line 131; `ExportClip::blank` ~line 243)
- Modify: `src/crates/concat-export/src/flatten.rs` (`export_animations`; the media `ExportClip` literal ~line 92; tests at ~line 205)
- Modify: `src/crates/concat-export/src/resolve.rs` (after `engine_clip.animation = …` ~line 259)
- Modify: `src/crates/concat-host/src/titles.rs` (the title `ExportClip` literal ~line 147)
- Test: `src/crates/concat-host/tests/export.rs`

**Interfaces:**
- Consumes:
  - from Task 1: `model::Clip::{animation_in, animation_out}`, `ClipAnimation`, `AnimationSlot`, `Command::SetClipAnimation`
  - from Task 2: `concat_core::motion::Played` and `concat_core::timeline::Clip::{entrance, exit}`
- Produces:
  - `concat_export::ExportAnimation { pub id: String, pub seconds: f64 }`, which is serde
  - `ExportClip::{entrance, exit}: Option<ExportAnimation>`
  - `concat_export::flatten::export_animations(clip: &model::Clip) -> (Option<ExportAnimation>, Option<ExportAnimation>)`

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/crates/concat-export/src/flatten.rs`:

```rust
    #[test]
    fn the_flattener_holds_the_windows_inside_the_clip() {
        use concat_project::model::{Clip, ClipAnimation, ClipKind};
        let mut clip = Clip::blank("c1", "t1", ClipKind::Video, "clip", 0.0, 2.0);
        clip.animation_in = Some(ClipAnimation {
            id: "zoom-in".to_owned(),
            duration: 1.5,
        });
        clip.animation_out = Some(ClipAnimation {
            id: "zoom-out".to_owned(),
            duration: 1.5,
        });
        let (entrance, exit) = export_animations(&clip);
        assert_eq!(entrance.expect("in").seconds, 1.5);
        assert_eq!(exit.expect("out").seconds, 0.5);
    }

    #[test]
    fn a_length_that_is_not_a_positive_number_plays_nothing() {
        use concat_project::model::{Clip, ClipAnimation, ClipKind};
        let mut clip = Clip::blank("c1", "t1", ClipKind::Video, "clip", 0.0, 2.0);
        clip.animation_in = Some(ClipAnimation {
            id: "zoom-in".to_owned(),
            duration: f64::NAN,
        });
        clip.animation_out = Some(ClipAnimation {
            id: "zoom-out".to_owned(),
            duration: -1.0,
        });
        assert_eq!(export_animations(&clip), (None, None));
    }
```

Add to `src/crates/concat-host/tests/export.rs`, after `removing_silences_leaves_only_the_sound`:

```rust
/// Clip animations reach the picture: a red clip that fades in over its
/// first second and out over its last starts black, is red in the middle,
/// and is black again at its end.
#[test]
fn an_animation_reaches_the_picture() {
    use concat_project::commands::AnimationSlot;
    use concat_project::model::ClipAnimation;

    let scratch = Scratch::new("animations");
    let red_path = scratch.path().join("red.mp4");
    let red = [220, 30, 30];
    solid(&red_path, red);
    let mut studio = Studio::new(scratch.path(), "Animations", video(WIDTH, HEIGHT, 30, 1));
    let media = studio.import(&red_path);
    let clip = studio
        .apply(Command::AddClipAtFirstFree {
            media_id: media,
            start: 0.0,
        })
        .expect("placed");
    for (slot, id) in [(AnimationSlot::In, "fade-in"), (AnimationSlot::Out, "fade-out")] {
        studio.apply(Command::SetClipAnimation {
            clip_id: clip.clone(),
            slot,
            animation: Some(ClipAnimation {
                id: id.to_owned(),
                duration: 1.0,
            }),
        });
    }
    let exported = studio.export("fade in and out");
    exported.expect_colours(0.0, &[((0.5, 0.5), [0, 0, 0])]);
    exported.expect_colours(1.5, &[((0.5, 0.5), red)]);
    exported.expect_colours(2.95, &[((0.5, 0.5), [0, 0, 0])]);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `wcargo test -p concat-export --lib flatten 2>&1 | tail -15`
Expected: a compile error, because `export_animations` is not found.

- [ ] **Step 3: Add `ExportAnimation` and the fields**

In `src/crates/concat-export/src/lib.rs`, next to `pub struct TransitionSpec` and with the same derives it has:

```rust
/// A preset on one end of an exported clip: which, and for how long,
/// already held inside the clip. See `concat_core::motion`.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
pub struct ExportAnimation {
    /// The preset's id.
    pub id: String,
    /// Seconds it covers.
    pub seconds: f64,
}
```

Adjust the derive list to match `TransitionSpec`'s, keeping `PartialEq` and `Debug`, since the flatten test compares values.

In `pub struct ExportClip`, after `pub animation: Vec<ExportKey>,`:

```rust
    /// The preset over the clip's first seconds, if any.
    #[serde(default)]
    pub entrance: Option<ExportAnimation>,
    /// The preset over its last seconds, if any.
    #[serde(default)]
    pub exit: Option<ExportAnimation>,
```

In `ExportClip::blank`, after `animation: Vec::new(),`, add `entrance: None,` and `exit: None,`.

- [ ] **Step 4: Flatten them**

In `src/crates/concat-export/src/flatten.rs`, change the import to `use crate::{ClipKind, ExportAnimation, ExportClip, ExportKey, TransitionSpec};` and add after `export_keys`:

```rust
/// The clip's In and Out as the engine plays them: the In held to the
/// clip, the Out to what the In leaves, and a length that is not a
/// positive number read as no animation - a hand-edited file degrades to
/// a clip that stands still.
pub fn export_animations(
    clip: &concat_project::model::Clip,
) -> (Option<ExportAnimation>, Option<ExportAnimation>) {
    let fit = |animation: &Option<concat_project::model::ClipAnimation>, room: f64| {
        animation
            .as_ref()
            .filter(|animation| animation.duration.is_finite() && animation.duration > 0.0)
            .map(|animation| ExportAnimation {
                id: animation.id.clone(),
                seconds: animation.duration.min(room.max(0.0)),
            })
    };
    let entrance = fit(&clip.animation_in, clip.duration);
    let taken = entrance.as_ref().map_or(0.0, |animation| animation.seconds);
    let exit = fit(&clip.animation_out, clip.duration - taken);
    (entrance, exit)
}
```

In `flatten_timeline_in`, in the media clip's closure, add `let (entrance, exit) = export_animations(clip);` on the line before `Some(ExportClip {`. Add the fields after `animation: export_keys(clip),`:

```rust
                entrance,
                exit,
```

The layer branch keeps no animation: `ExportClip::blank` supplies `None`.

- [ ] **Step 5: Titles too**

In `src/crates/concat-host/src/titles.rs`, before `out.push(TitleClip {`, add:

```rust
            let (entrance, exit) = concat_export::flatten::export_animations(clip);
```

Add the fields after `animation: concat_export::flatten::export_keys(clip),`:

```rust
                    entrance,
                    exit,
```

- [ ] **Step 6: Hand them to the engine**

In `src/crates/concat-export/src/resolve.rs`, after `engine_clip.animation = animation_of(&clip.animation);`:

```rust
        let played = |animation: &crate::ExportAnimation| concat_core::motion::Played {
            id: animation.id.clone(),
            seconds: animation.seconds,
        };
        engine_clip.entrance = clip.entrance.as_ref().map(played);
        engine_clip.exit = clip.exit.as_ref().map(played);
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `wcargo test -p concat-export --lib 2>&1 | tail -15`
Expected: every test passes, including the 2 new ones.

Run: `wcargo test -p concat-host --test export an_animation_reaches_the_picture 2>&1 | tail -15`
Expected: `test an_animation_reaches_the_picture ... ok`.

Run: `wcargo clippy -p concat-export -p concat-host --all-targets -- -D warnings 2>&1 | tail -5`
Expected: no warnings.

Run: `wcargo fmt --all`, then `wcargo fmt --all --check && echo FMT_CLEAN`
Expected: `FMT_CLEAN`.

- [ ] **Step 8: Commit**

```bash
wgit add src/crates/concat-export src/crates/concat-host
# msg.txt: "feat(export): clip animations reach the preview and the export" + trailer
wgit commit -F .superpowers/sdd/2026-09-29-clip-animations/msg.txt
```

---

### Task 4: The tab's arithmetic

**Files:**
- Create: `src/crates/concat/src/panes/animations.rs`
- Modify: `src/crates/concat/src/panes/mod.rs` (add `pub mod animations;`)

**Interfaces:**
- Consumes:
  - from Task 1: `Clip::{animation_in, animation_out, duration, start}`, `MIN_ANIMATION` and `AnimationSlot`
  - from Task 2: `motion::{PRESETS, Preset, Side, Group}`
- Produces (in `crate::panes::animations`):
  - `DEFAULT_LENGTH: f64 = 0.5`
  - `side_of(slot: AnimationSlot) -> Side`
  - `offered(slot: AnimationSlot) -> Vec<&'static Preset>`
  - `current(clip: &Clip, slot: AnimationSlot) -> Option<&ClipAnimation>`
  - `room(clip: &Clip, slot: AnimationSlot) -> f64`
  - `length_range(clip: &Clip, slot: AnimationSlot) -> (f64, f64)`
  - `starting_length(clip: &Clip, slot: AnimationSlot) -> f64`
  - `preview_window(clip: &Clip, slot: AnimationSlot, seconds: f64) -> (f64, f64)`

- [ ] **Step 1: Write the failing tests**

Create `src/crates/concat/src/panes/animations.rs`:

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

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
        assert!(ins[first_effect..].iter().all(|preset| preset.group == Group::Effects));
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
        assert_eq!(starting_length(&clip(5.0), AnimationSlot::In), DEFAULT_LENGTH);
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
```

Add `pub mod animations;` to `src/crates/concat/src/panes/mod.rs`, in alphabetical order with the others.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `wcargo test -p concat --lib panes::animations 2>&1 | tail -15`
Expected: a compile error, because `offered`, `room` and the other functions are not found.

- [ ] **Step 3: Implement**

Put this above the tests in `src/crates/concat/src/panes/animations.rs`:

```rust
//! The Animations tab's arithmetic: which presets each end offers, how
//! long an animation may be, and which stretch of the timeline the
//! preview plays after a pick. No window and no project: the studio asks,
//! and paints what comes back.

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
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `wcargo test -p concat --lib panes::animations 2>&1 | tail -15`
Expected: 5 passed.

Run: `wcargo clippy -p concat --all-targets -- -D warnings 2>&1 | tail -5`
Expected: no warnings. If clippy reports dead code, because nothing calls these until Task 5, add `#![allow(dead_code)] // wired in Task 5` at the top of the file and remove it in Task 5.

- [ ] **Step 5: Commit**

```bash
wgit add src/crates/concat/src/panes
# msg.txt: "feat(window): the Animations tab's arithmetic" + trailer
wgit commit -F .superpowers/sdd/2026-09-29-clip-animations/msg.txt
```

---

### Task 5: The Animations tab

**Files:**
- Create: `src/crates/concat/ui/inspector/animations-panel.slint`
- Modify: `src/crates/concat/ui/timeline/model.slint` (`SelectedClipData` ~line 280; `ClipData` ~line 115)
- Modify: `src/crates/concat/ui/workspace/config-pane.slint` (tabs ~line 225, sections ~line 263, `jump`, the panel list ~line 540, props and callbacks ~line 203)
- Modify: `src/crates/concat/ui/workspace/seat.slint` (pass-through ~line 134 and ~line 257)
- Modify: `src/crates/concat/ui/editor.slint` (global props ~line 155, callbacks ~line 273)
- Modify: `src/crates/concat/ui/timeline/lanes.slint` (the animation bars)
- Modify: `src/crates/concat/src/studio.rs` (Models, fill, `SelectedClipData`, `ClipData`, `pick_animation`, `set_animation_length`, `replay_animation`, `play_window`, `stop_at`, `duplicate`)
- Modify: `src/crates/concat/src/lib.rs` (set models ~line 216, callbacks ~line 1130)
- Modify: `src/crates/concat/locales/*.json` (28 keys × 14 files)

**Interfaces:**
- Consumes:
  - from Task 4: every `panes::animations` function
  - from Task 1: `Command::SetClipAnimation`, `AnimationSlot`, `ClipAnimation`
  - from Task 2: `Preset.label` (a full i18n key) and `Preset.effect`
- Produces (Slint):
  - `AnimationPresetData { id: string, name: string, effects: bool, glyph: Glyph, art: image }`
  - Editor global: `animation-presets-in` and `animation-presets-out`, both `[AnimationPresetData]`
  - callbacks `animation-pick(out: bool, id: string)`, `animation-length-set(out: bool, seconds: float)` and `animation-replay(out: bool)`
  - `SelectedClipData.{animation-in-id, animation-out-id: string; animation-in-length, animation-out-length, animation-in-room, animation-out-room: float}`
  - `ClipData.{animation-in, animation-out: float}`

- [ ] **Step 1: Write the failing test (the locales)**

The window's automated gate here is the locale test, and the tab's logic was tested in Task 4. Add the 28 keys to `en.json` only, then run the locale test and watch it fail for the 13 other files. Save this script as `.superpowers/sdd/2026-09-29-clip-animations/locales_task5.py`:

```python
import json, sys, pathlib
root = pathlib.Path(sys.argv[1])
KEYS = ["configPane.animations", "animations.in", "animations.out", "animations.none",
        "animations.basic", "animations.replay",
        "animations.preset.fadeIn", "animations.preset.zoomIn", "animations.preset.zoomFromBig",
        "animations.preset.slideFromLeft", "animations.preset.slideFromRight",
        "animations.preset.slideFromTop", "animations.preset.slideFromBottom",
        "animations.preset.rise", "animations.preset.spinIn", "animations.preset.pop",
        "animations.preset.shakeIn",
        "animations.preset.fadeOut", "animations.preset.zoomOut", "animations.preset.zoomToBig",
        "animations.preset.slideToLeft", "animations.preset.slideToRight",
        "animations.preset.slideToTop", "animations.preset.slideToBottom",
        "animations.preset.sink", "animations.preset.spinOut", "animations.preset.popOut",
        "animations.preset.shakeOut"]
T = {
 "en": ["Animations","In","Out","None","Basic","Play it again","Fade in","Zoom in","Zoom from big","Slide from left","Slide from right","Slide from top","Slide from bottom","Rise","Spin in","Pop","Shake in","Fade out","Zoom out","Zoom to big","Slide to left","Slide to right","Slide to top","Slide to bottom","Sink","Spin out","Pop out","Shake out"],
 "de": ["Animationen","Ein","Aus","Keine","Einfach","Erneut abspielen","Einblenden","Hineinzoomen","Von groß zoomen","Von links hereingleiten","Von rechts hereingleiten","Von oben hereingleiten","Von unten hereingleiten","Aufsteigen","Hineindrehen","Aufploppen","Hineinwackeln","Ausblenden","Herauszoomen","Zu groß zoomen","Nach links hinausgleiten","Nach rechts hinausgleiten","Nach oben hinausgleiten","Nach unten hinausgleiten","Absinken","Hinausdrehen","Wegploppen","Hinauswackeln"],
 "es": ["Animaciones","Entrada","Salida","Ninguna","Básicas","Reproducir de nuevo","Aparecer","Acercar","Desde grande","Deslizar desde la izquierda","Deslizar desde la derecha","Deslizar desde arriba","Deslizar desde abajo","Elevar","Girar al entrar","Pop","Sacudir al entrar","Desvanecer","Alejar","Hacia grande","Deslizar a la izquierda","Deslizar a la derecha","Deslizar hacia arriba","Deslizar hacia abajo","Hundir","Girar al salir","Pop de salida","Sacudir al salir"],
 "fa": ["انیمیشن‌ها","ورود","خروج","هیچ","پایه","پخش دوباره","ظاهر شدن تدریجی","بزرگ‌نمایی","از بزرگ","لغزش از چپ","لغزش از راست","لغزش از بالا","لغزش از پایین","بالا آمدن","چرخش ورودی","پاپ","لرزش ورودی","محو شدن تدریجی","کوچک‌نمایی","به بزرگ","لغزش به چپ","لغزش به راست","لغزش به بالا","لغزش به پایین","فرو رفتن","چرخش خروجی","پاپ خروجی","لرزش خروجی"],
 "fr": ["Animations","Entrée","Sortie","Aucune","Base","Rejouer","Fondu entrant","Zoom avant","Depuis grand","Glisser depuis la gauche","Glisser depuis la droite","Glisser depuis le haut","Glisser depuis le bas","Montée","Rotation entrante","Pop","Secousse entrante","Fondu sortant","Zoom arrière","Vers grand","Glisser vers la gauche","Glisser vers la droite","Glisser vers le haut","Glisser vers le bas","Descente","Rotation sortante","Pop sortant","Secousse sortante"],
 "hr": ["Animacije","Ulaz","Izlaz","Nijedna","Osnovno","Ponovno reproduciraj","Postupno pojavljivanje","Približavanje","Iz velikog","Klizanje slijeva","Klizanje zdesna","Klizanje odozgo","Klizanje odozdo","Uzdizanje","Ulazna rotacija","Iskakanje","Ulazno drmanje","Postupno nestajanje","Udaljavanje","U veliko","Klizanje ulijevo","Klizanje udesno","Klizanje prema gore","Klizanje prema dolje","Spuštanje","Izlazna rotacija","Izlazno iskakanje","Izlazno drmanje"],
 "it": ["Animazioni","Entrata","Uscita","Nessuna","Base","Riproduci di nuovo","Dissolvenza in entrata","Zoom avanti","Da grande","Scorri da sinistra","Scorri da destra","Scorri dall'alto","Scorri dal basso","Sali","Rotazione in entrata","Pop","Scossa in entrata","Dissolvenza in uscita","Zoom indietro","Verso grande","Scorri a sinistra","Scorri a destra","Scorri in alto","Scorri in basso","Scendi","Rotazione in uscita","Pop in uscita","Scossa in uscita"],
 "ja": ["アニメーション","イン","アウト","なし","基本","もう一度再生","フェードイン","ズームイン","拡大から","左からスライド","右からスライド","上からスライド","下からスライド","ライズ","スピンイン","ポップ","シェイクイン","フェードアウト","ズームアウト","拡大へ","左へスライド","右へスライド","上へスライド","下へスライド","シンク","スピンアウト","ポップアウト","シェイクアウト"],
 "ko": ["애니메이션","인","아웃","없음","기본","다시 재생","페이드 인","줌 인","크게에서","왼쪽에서 슬라이드","오른쪽에서 슬라이드","위에서 슬라이드","아래에서 슬라이드","떠오르기","회전 인","팝","흔들며 인","페이드 아웃","줌 아웃","크게로","왼쪽으로 슬라이드","오른쪽으로 슬라이드","위로 슬라이드","아래로 슬라이드","가라앉기","회전 아웃","팝 아웃","흔들며 아웃"],
 "pt-BR": ["Animações","Entrada","Saída","Nenhuma","Básicas","Reproduzir de novo","Surgir","Aproximar","Do grande","Deslizar da esquerda","Deslizar da direita","Deslizar de cima","Deslizar de baixo","Subir","Girar ao entrar","Pop","Tremer ao entrar","Sumir","Afastar","Para grande","Deslizar para a esquerda","Deslizar para a direita","Deslizar para cima","Deslizar para baixo","Afundar","Girar ao sair","Pop de saída","Tremer ao sair"],
 "ru": ["Анимации","Появление","Исчезновение","Нет","Основные","Воспроизвести снова","Плавное появление","Приближение","Из крупного","Въезд слева","Въезд справа","Въезд сверху","Въезд снизу","Подъём","Вращение при входе","Всплытие","Тряска при входе","Плавное исчезновение","Отдаление","В крупный","Уход влево","Уход вправо","Уход вверх","Уход вниз","Погружение","Вращение при выходе","Схлопывание","Тряска при выходе"],
 "tr": ["Animasyonlar","Giriş","Çıkış","Yok","Temel","Yeniden oynat","Belirerek gir","Yakınlaş","Büyükten","Soldan kay","Sağdan kay","Yukarıdan kay","Aşağıdan kay","Yüksel","Dönerek gir","Pop","Sallanarak gir","Kaybolarak çık","Uzaklaş","Büyüğe","Sola kay","Sağa kay","Yukarı kay","Aşağı kay","Bat","Dönerek çık","Pop çıkış","Sallanarak çık"],
 "zh-Hans": ["动画","入场","出场","无","基础","再次播放","渐显","放大","由大缩小","从左滑入","从右滑入","从上滑入","从下滑入","上升","旋转入场","弹出","抖动入场","渐隐","缩小","放大消失","向左滑出","向右滑出","向上滑出","向下滑出","下沉","旋转出场","弹出消失","抖动出场"],
 "zh-TW": ["動畫","入場","出場","無","基本","再播放一次","漸顯","放大","由大縮小","從左滑入","從右滑入","從上滑入","從下滑入","上升","旋轉入場","彈出","抖動入場","漸隱","縮小","放大消失","向左滑出","向右滑出","向上滑出","向下滑出","下沉","旋轉出場","彈出消失","抖動出場"],
}
only = sys.argv[2:]  # e.g. "en" to add English alone (the RED step)
for code, values in T.items():
    if only and code not in only:
        continue
    assert len(values) == len(KEYS), code
    path = root / f"{code}.json"
    raw = path.read_bytes().decode("utf-8")
    crlf = "\r\n" in raw
    data = json.loads(raw)
    was_sorted = list(data) == sorted(data)
    for key, value in zip(KEYS, values):
        data[key] = value
    if was_sorted:
        data = dict(sorted(data.items()))
    text = json.dumps(data, ensure_ascii=False, indent=2) + "\n"
    if crlf:
        text = text.replace("\n", "\r\n")
    path.write_bytes(text.encode("utf-8"))
```

Before running it, open `en.json` and check how it is laid out: two-space indent, and whether the keys are sorted. If it is not plain `json.dumps` output (for example, a metadata key that must stay first), change the script so the diff touches only the new lines. `git diff --stat` on the locales should then show about +28 lines per file.

Run: `python3 .superpowers/sdd/2026-09-29-clip-animations/locales_task5.py src/crates/concat/locales en`

Run: `wcargo test -p concat --lib i18n 2>&1 | tail -15`
Expected: FAIL in `every_shipped_locale_parses_names_itself_and_keys_off_the_inventory` with `de.json lacks [...]` (or the first locale alphabetically).

- [ ] **Step 2: Add the other 13 translations and watch the test pass**

Run: `python3 .superpowers/sdd/2026-09-29-clip-animations/locales_task5.py src/crates/concat/locales de es fa fr hr it ja ko pt-BR ru tr zh-Hans zh-TW`

Run: `wcargo test -p concat --lib i18n 2>&1 | tail -15`
Expected: every i18n test passes.

- [ ] **Step 3: The panel**

Create `src/crates/concat/ui/inspector/animations-panel.slint`:

```slint
// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

import { I18n } from "../i18n.slint";
import { Theme } from "../theme/theme.slint";
import { Glyph, Icon } from "../icons.slint";
import { IconButton } from "../primitives/panel.slint";
import { SegmentedControl } from "../primitives/segmented-control.slint";
import { Param, ParamFormat, Subhead } from "controls.slint";
import { SelectedClipData } from "../timeline/model.slint";

/// One preset the Animations tab offers. Published by Rust from
/// `concat_core::motion::PRESETS`; the id is what comes back on a click,
/// and it is stable forever because project files store it.
export struct AnimationPresetData {
    id: string,
    name: string,
    /// On the Effects shelf rather than Basic.
    effects: bool,
    /// The card's mark, for a motion preset.
    glyph: Glyph,
    /// The card's still, for an effect preset: its effect's own. Empty
    /// while that is still being drawn.
    art: image,
}

// A card in the grid: a mark or a still, and the name under it.
component AnimationCard inherits Rectangle {
    in property <string> name;
    in property <Glyph> glyph;
    in property <image> art;
    in property <bool> chosen;
    callback clicked();

    height: 68px;
    border-radius: Theme.r-sm;
    background: touch.has-hover ? Theme.field-hover : Theme.field;
    border-width: root.chosen ? 2px : 1px;
    border-color: root.chosen ? Theme.accent : Theme.line;

    VerticalLayout {
        padding: 6px;
        spacing: 4px;
        Rectangle {
            vertical-stretch: 1;
            if root.art.width > 0: Image {
                width: parent.width;
                height: parent.height;
                source: root.art;
                image-fit: cover;
            }
            if root.art.width == 0: Icon {
                x: (parent.width - self.width) / 2;
                y: (parent.height - self.height) / 2;
                glyph: root.glyph;
                size: 20px;
                tint: root.chosen ? Theme.accent : Theme.fg-muted;
            }
        }
        Text {
            text: root.name;
            color: Theme.fg;
            font-size: Theme.fs-xs;
            horizontal-alignment: center;
            overflow: elide;
        }
    }

    touch := TouchArea {
        clicked => { root.clicked(); }
    }
}

// A shelf of cards, three to a row.
component AnimationGrid inherits VerticalLayout {
    in property <[AnimationPresetData]> presets;
    in property <string> chosen;
    in property <bool> with-none;
    callback picked(id: string);

    property <length> card-w: (self.width - 2 * Theme.pad - 2 * 6px) / 3;

    padding-left: Theme.pad;
    padding-right: Theme.pad;

    Rectangle {
        height: self.rows * 74px;
        property <int> none: root.with-none ? 1 : 0;
        property <int> count: root.none + root.shown.length;
        property <int> rows: Math.ceil(self.count / 3);
        property <[AnimationPresetData]> shown: root.presets;

        if root.with-none: AnimationCard {
            x: 0;
            y: 0;
            width: root.card-w;
            name: I18n.t("animations.none");
            glyph: Glyph.close;
            chosen: root.chosen == "";
            clicked => { root.picked(""); }
        }
        for preset[index] in parent.shown: AnimationCard {
            property <int> slot: index + parent.none;
            x: Math.mod(self.slot, 3) * (root.card-w + 6px);
            y: Math.floor(self.slot / 3) * 74px;
            width: root.card-w;
            name: preset.name;
            glyph: preset.glyph;
            art: preset.art;
            chosen: preset.id == root.chosen;
            clicked => { root.picked(preset.id); }
        }
    }
}

/// The Animations tab: an In | Out switch, the presets for that end on two
/// shelves, and the length of the one chosen.
export component AnimationsPanel inherits VerticalLayout {
    in property <SelectedClipData> selected;
    in property <[AnimationPresetData]> presets-in;
    in property <[AnimationPresetData]> presets-out;

    callback pick(out: bool, id: string);
    callback length-set(out: bool, seconds: float);
    callback replay(out: bool);

    // Assigned, never bound: the segmented control writes to it.
    property <int> side;
    property <bool> out: root.side == 1;
    property <[AnimationPresetData]> presets: root.out ? root.presets-out : root.presets-in;
    property <string> chosen: root.out ? root.selected.animation-out-id : root.selected.animation-in-id;
    // Seeded rather than bound, for the reason given above config-pane's
    // panels: the Param writes to its own value as it is dragged.
    property <float> length;
    function seed() {
        root.length = root.out ? root.selected.animation-out-length : root.selected.animation-in-length;
    }
    init => { root.seed(); }
    changed selected => { root.seed(); }
    changed side => { root.seed(); }

    spacing: 8px;
    padding-top: 8px;
    padding-bottom: 12px;

    HorizontalLayout {
        padding-left: Theme.pad;
        padding-right: Theme.pad;
        SegmentedControl {
            options: [I18n.t("animations.in"), I18n.t("animations.out")];
            current <=> root.side;
        }
    }

    Subhead { title: I18n.t("animations.basic"); }
    AnimationGrid {
        presets: root.presets;
        with-none: true;
        chosen: root.chosen;
        picked(id) => { root.pick(root.out, id); }
    }

    // Task 6 adds the Effects shelf here.

    if root.chosen != "": HorizontalLayout {
        Param {
            horizontal-stretch: 1;
            label: I18n.t("common.duration");
            value <=> root.length;
            minimum: Math.min(0.1, root.out ? root.selected.animation-out-room : root.selected.animation-in-room);
            maximum: root.out ? root.selected.animation-out-room : root.selected.animation-in-room;
            step: 0.05;
            default-value: 0.5;
            fmt: ParamFormat.seconds2;
            changed(v) => { root.length-set(root.out, v); }
        }
        IconButton {
            glyph: Glyph.play;
            label: I18n.t("animations.replay");
            clicked => { root.replay(root.out); }
        }
    }
}
```

Check each name this file uses against the real components before the first build:
- `IconButton`'s properties (`glyph`, `label`, `clicked`) in `primitives/panel.slint`
- `Subhead { title }` and `Param` in `inspector/controls.slint`
- `Icon`'s `tint`/`size` in `icons.slint`
- the Theme names `r-sm`, `field`, `field-hover`, `fs-xs`, `pad`, `line`, `accent`, `fg`, `fg-muted`

Adjust to what exists; Slint names every mismatch in its error.

- [ ] **Step 4: The models**

In `src/crates/concat/ui/timeline/model.slint`:
- In `SelectedClipData`, after `transition-duration: float,`:

```slint
    // the animations at either end; an empty id means none. A length is
    // the slider's value, and a room its ceiling: the clip less the other end.
    animation-in-id: string,
    animation-in-length: float,
    animation-in-room: float,
    animation-out-id: string,
    animation-out-length: float,
    animation-out-room: float,
```

- In `ClipData`, after `transition-duration: float,`:

```slint
    /// Seconds of In and Out animation at the clip's ends; zero for none.
    /// Drawn as a bar along the top of each end.
    animation-in: float,
    animation-out: float,
```

- [ ] **Step 5: Config pane, seat and editor**

In `src/crates/concat/ui/workspace/config-pane.slint`:

- Import: `import { AnimationPresetData, AnimationsPanel } from "../inspector/animations-panel.slint";`
- Properties and callbacks, next to `callback transition-duration-set(seconds: float);`:

```slint
    in property <[AnimationPresetData]> animation-presets-in;
    in property <[AnimationPresetData]> animation-presets-out;
    callback animation-pick(out: bool, id: string);
    callback animation-length-set(out: bool, seconds: float);
    callback animation-replay(out: bool);
```

- `tabs`: add `"Animations"` at the end of the text, image and video lists, and change nothing for layer and audio:

```slint
    property <[string]> tabs: root.is-text ? ["Text", "Effects", "Animations"]
        : root.is-layer ? ["Filter", "Effects"]
        : root.is-image ? ["Video", "Effects", "Animations"]
        : root.is-audio ? ["Audio"]
        : ["Video", "Audio", "Effects", "Animations"];
```

- `tabs-shown`: the same, adding `I18n.t("configPane.animations")` at the end of the text, image and video lists.
- Update the comment above `is-text`: after "three tabs at most", add a sentence saying that Animations is the one tab past that rule, because every picture clip needs it and it has no sections of its own.
- `sections` and `sections-shown`: add a branch before the final `: ["Layer"]` or `: [I18n.t("common.layer")]`:
  - `: root.heading == "Animations" ? ["Animations"]`
  - `: root.heading == "Animations" ? [I18n.t("configPane.animations")]`
- `jump()`: `"Effects"` must stay the Effects tab and not the last one. Change `root.tab = root.tabs.length - 1;` to `root.tab = root.is-text || root.is-image || root.is-layer ? 1 : 2;`.
- Panel list: after the `if root.showing == "Text": TextPanel { … }` block:

```slint
                if root.showing == "Animations": AnimationsPanel {
                    selected: root.selected;
                    presets-in: root.animation-presets-in;
                    presets-out: root.animation-presets-out;
                    pick(out, id) => { root.animation-pick(out, id); }
                    length-set(out, seconds) => { root.animation-length-set(out, seconds); }
                    replay(out) => { root.animation-replay(out); }
                }
```

In `src/crates/concat/ui/editor.slint`, next to `in-out property <[CatalogueEntryData]> catalogue-transitions;`, add the two `in-out property <[AnimationPresetData]> animation-presets-in;` and `…-out;` lines, importing the type from `inspector/animations-panel.slint`. Add the three callbacks next to `callback transition-duration-set(seconds: float);`.

In `src/crates/concat/ui/workspace/seat.slint`, where the ConfigPane gets `transition-duration-set(seconds) => { Editor.transition-duration-set(seconds); }`, add:

```slint
            animation-presets-in: Editor.animation-presets-in;
            animation-presets-out: Editor.animation-presets-out;
            animation-pick(out, id) => { Editor.animation-pick(out, id); }
            animation-length-set(out, seconds) => { Editor.animation-length-set(out, seconds); }
            animation-replay(out) => { Editor.animation-replay(out); }
```

- [ ] **Step 6: The timeline bars**

In `src/crates/concat/ui/timeline/lanes.slint`, in the clip component (the one declaring `property <bool> has-transition` ~line 199), add just before the first `if root.has-transition && root.width > 10px:` block:

```slint
    // The In and Out animations: a bar along the top edge of each end, as
    // wide as its window, so an animated clip reads as one at a glance.
    if root.item.animation-in > 0: Rectangle {
        x: 0;
        y: 0;
        width: Math.min(root.width, root.item.animation-in * root.px-per-second);
        height: 3px;
        background: Theme.accent;
        opacity: 0.85;
    }
    if root.item.animation-out > 0: Rectangle {
        x: root.width - self.width;
        y: 0;
        width: Math.min(root.width, root.item.animation-out * root.px-per-second);
        height: 3px;
        background: Theme.accent;
        opacity: 0.85;
    }
```

`root.px-per-second` is the name the component already uses for `transition-w`.

- [ ] **Step 7: Rust: publish, pick, length, replay, play a window, duplicate**

In `src/crates/concat/src/studio.rs`:

1. **Models**: in `pub struct Models`, after `catalogue_transitions`:

```rust
    /// The Animations tab's two grids, one per end, in the interface's
    /// language; see `fill_animation_presets`.
    pub animation_presets_in: Rc<VecModel<AnimationPresetData>>,
    pub animation_presets_out: Rc<VecModel<AnimationPresetData>>,
```

   Initialise both with `Rc::new(VecModel::default())` next to `catalogue_transitions` (~line 654).

2. **Fill function**, added near the catalogue sync:

```rust
/// The mark a motion preset's card wears: the direction or the kind of
/// its movement.
fn animation_glyph(id: &str) -> Glyph {
    match id {
        "fade-in" | "fade-out" => Glyph::Eye,
        "zoom-in" | "zoom-out" => Glyph::Maximize,
        "zoom-from-big" | "zoom-to-big" => Glyph::Fit,
        "slide-from-left" | "slide-to-right" => Glyph::ChevronRight,
        "slide-from-right" | "slide-to-left" => Glyph::ChevronLeft,
        "slide-from-top" | "slide-to-bottom" => Glyph::ChevronDown,
        "slide-from-bottom" | "slide-to-top" | "rise" => Glyph::ChevronUp,
        "sink" => Glyph::ChevronDown,
        "spin-in" | "spin-out" => Glyph::Rotate,
        "pop" | "pop-out" => Glyph::Sparkle,
        "shake-in" | "shake-out" => Glyph::Waveform,
        _ => Glyph::Sparkle,
    }
}

/// Publishes the Basic grid of each end, named in the current language.
/// Task 6 adds the Effects grids beside them.
fn fill_animation_presets(models: &Models) {
    use crate::panes::animations;
    use concat_core::motion::Group;
    use concat_project::commands::AnimationSlot;
    let art_of = |effect: &str| {
        models
            .catalogue_effects
            .iter()
            .find(|entry| entry.id == effect)
            .map(|entry| entry.art)
            .unwrap_or_default()
    };
    for (slot, model) in [
        (AnimationSlot::In, &models.animation_presets_in),
        (AnimationSlot::Out, &models.animation_presets_out),
    ] {
        let rows: Vec<AnimationPresetData> = animations::offered(slot)
            .into_iter()
            .filter(|preset| preset.group == Group::Basic)
            .map(|preset| AnimationPresetData {
                id: preset.id.into(),
                name: i18n::t(preset.label).into(),
                effects: preset.group == Group::Effects,
                glyph: animation_glyph(preset.id),
                art: preset.effect.map(|ramp| art_of(ramp.effect)).unwrap_or_default(),
            })
            .collect();
        model.set_vec(rows);
    }
}
```

   Call `fill_animation_presets(models);` directly after `sync(&models.catalogue_transitions, entries);` (~line 8702), in the function that republishes the shelves. That function runs again on a language change, so the names follow. If `models` there is not `&Models`, pass what the function has. Import `AnimationPresetData` and `Glyph` with the other generated Slint types at the top of the file.

3. **`SelectedClipData`**: in the literal at ~line 8410, after `transition_duration: …,`:

```rust
            animation_in_id: clip
                .animation_in
                .as_ref()
                .map(|animation| animation.id.as_str())
                .unwrap_or_default()
                .into(),
            animation_in_length: clip
                .animation_in
                .as_ref()
                .map_or(0.5, |animation| animation.duration as f32),
            animation_in_room: crate::panes::animations::room(clip, AnimationSlot::In) as f32,
            animation_out_id: clip
                .animation_out
                .as_ref()
                .map(|animation| animation.id.as_str())
                .unwrap_or_default()
                .into(),
            animation_out_length: clip
                .animation_out
                .as_ref()
                .map_or(0.5, |animation| animation.duration as f32),
            animation_out_room: crate::panes::animations::room(clip, AnimationSlot::Out) as f32,
```

4. **`ClipData`**: in the literal at ~line 7171, after `transition_duration: …,`:

```rust
                        animation_in: clip
                            .animation_in
                            .as_ref()
                            .map_or(0.0, |animation| animation.duration as f32),
                        animation_out: clip
                            .animation_out
                            .as_ref()
                            .map_or(0.0, |animation| animation.duration as f32),
```

   Run `grep -n "ClipData {\|SelectedClipData {" src/crates/concat/src/*.rs src/crates/concat/src/**/*.rs` and give every other literal the new fields, or `..Default::default()` where the literal already ends with one.

5. **Stopping playback at a set point**: add `stop_at: Option<f32>,` to `pub struct Studio`, with a doc line: `/// Where playback stops by itself, when a preview asked for a stretch rather than the rest of the timeline.`. Set `stop_at: None,` wherever `playing: false,` is initialised. In `play_toggle`'s timer closure, replace `let end = studio.duration();` with:

```rust
                        let end = studio
                            .stop_at
                            .map_or(studio.duration(), |stop| stop.min(studio.duration()));
```

   In `pause`, add `self.stop_at = None;`.

6. **The studio's three calls**, next to `set_transition_duration`:

```rust
    /// Picks `id` for one end of the selected clip, or clears that end for
    /// an empty id, then plays that end in the viewer so the pick is seen
    /// at once. An end that had a preset keeps its length.
    pub fn pick_animation(&mut self, slot: AnimationSlot, id: &str) {
        use crate::panes::animations;
        let Some(clip_id) = self.sole_selection() else {
            return;
        };
        let Some(clip) = self.clip(&clip_id).cloned() else {
            return;
        };
        let animation = (!id.is_empty()).then(|| ClipAnimation {
            id: id.to_owned(),
            duration: animations::starting_length(&clip, slot),
        });
        let picked = animation.is_some();
        self.apply(Command::SetClipAnimation {
            clip_id: clip_id.clone(),
            slot,
            animation,
        });
        if picked {
            self.replay_animation(slot);
        }
    }

    /// Sets the length of one end of the selected clip's animation, as one
    /// move of the slider's gesture, so a drag is one undo step.
    pub fn set_animation_length(&mut self, slot: AnimationSlot, seconds: f64) {
        let Some(clip_id) = self.sole_selection() else {
            return;
        };
        let Some(current) = self
            .clip(&clip_id)
            .and_then(|clip| crate::panes::animations::current(clip, slot).cloned())
        else {
            return;
        };
        let gesture = format!("animation-{clip_id}-{slot:?}");
        self.apply_within(
            &gesture,
            Command::SetClipAnimation {
                clip_id,
                slot,
                animation: Some(ClipAnimation {
                    duration: seconds,
                    ..current
                }),
            },
        );
    }

    /// Plays the selected clip's animation on one end in the viewer.
    pub fn replay_animation(&mut self, slot: AnimationSlot) {
        use crate::panes::animations;
        let Some(clip_id) = self.sole_selection() else {
            return;
        };
        let Some(clip) = self.clip(&clip_id).cloned() else {
            return;
        };
        let Some(seconds) = animations::current(&clip, slot).map(|a| a.duration) else {
            return;
        };
        let (from, to) = animations::preview_window(&clip, slot, seconds);
        self.play_window(from as f32, to as f32);
    }

    /// Plays `from..to` of the timeline in the viewer and stops at `to`.
    pub fn play_window(&mut self, from: f32, to: f32) {
        if self.playing {
            self.pause();
        }
        self.seek(from.max(0.0));
        self.play_toggle();
        self.stop_at = Some(to);
    }
```

   Import `AnimationSlot` from `concat_project::commands` and `ClipAnimation` from `concat_project::model` in the file's existing `use` lists.

7. **Duplicate**: in `duplicate`:
   - In the text branch, after `self.selection = vec![id];`, add `self.copy_animations(source, &id);` inside the `if let Some(id) = created` block.
   - In the media branch, after the existing `commands.push(Command::UpdateClip { … });`:

```rust
        for (slot, animation) in [
            (AnimationSlot::In, &source.animation_in),
            (AnimationSlot::Out, &source.animation_out),
        ] {
            if animation.is_some() {
                commands.push(Command::SetClipAnimation {
                    clip_id: created.clone(),
                    slot,
                    animation: animation.clone(),
                });
            }
        }
```

   Add the helper next to `duplicate`:

```rust
    /// Gives `to` the animations `from` has.
    fn copy_animations(&mut self, from: &Clip, to: &str) {
        for (slot, animation) in [
            (AnimationSlot::In, &from.animation_in),
            (AnimationSlot::Out, &from.animation_out),
        ] {
            if animation.is_some() {
                self.apply(Command::SetClipAnimation {
                    clip_id: to.to_owned(),
                    slot,
                    animation: animation.clone(),
                });
            }
        }
    }
```

In `src/crates/concat/src/lib.rs`:
- Next to `editor.set_catalogue_transitions(…)` (~line 216):

```rust
        editor.set_animation_presets_in(ModelRc::from(models.animation_presets_in.clone()));
        editor.set_animation_presets_out(ModelRc::from(models.animation_presets_out.clone()));
```

- Next to `editor.on_transition_duration_set(…)` (~line 1130):

```rust
    let slot = |out: bool| {
        if out {
            concat_project::commands::AnimationSlot::Out
        } else {
            concat_project::commands::AnimationSlot::In
        }
    };
    editor.on_animation_pick(on_window!(|state, out: bool, id: slint::SharedString| {
        state.pick_animation(slot(out), &id);
    }));
    editor.on_animation_length_set(on_window!(|state, out: bool, seconds: f32| {
        state.set_animation_length(slot(out), seconds as f64);
    }));
    editor.on_animation_replay(on_window!(|state, out: bool| {
        state.replay_animation(slot(out));
    }));
```

`on_window!` is the macro the neighbouring callbacks use. If it can't capture the `slot` closure, inline the `if out { … } else { … }` in each callback.

If Task 4 added `#![allow(dead_code)]` to `panes/animations.rs`, remove it now.

- [ ] **Step 8: Build, test and lint**

Run: `wcargo test -p concat --lib 2>&1 | tail -20`
Expected: every test passes, including i18n and `panes::animations`. The known flaky pair (`format::phrases_are_coarse` and `speech::a_voice_name_reads...`, which race the i18n `select("de")` tests) may fail. If they do, rerun them alone with `-- --test-threads=1 format speech` and record them in the ledger as pre-existing.

Run: `wcargo clippy -p concat --all-targets -- -D warnings 2>&1 | tail -5`
Expected: no warnings.

Run: `wcargo fmt --all`, then `wcargo fmt --all --check && echo FMT_CLEAN`
Expected: `FMT_CLEAN`.

- [ ] **Step 9: Commit**

```bash
wgit add src/crates/concat
# msg.txt: "feat(window): the Animations tab" + trailer
wgit commit -F .superpowers/sdd/2026-09-29-clip-animations/msg.txt
```

---

### Task 6: Effect presets

**Files:**
- Create: `src/crates/concat-effects/packages/concat.glitch/effect.toml`, `effect.wgsl`, `fixtures.toml`
- Modify: `src/crates/concat-core/src/motion.rs` (12 Effects entries)
- Create: `src/crates/concat-export/src/animations.rs`
- Modify: `src/crates/concat-export/src/lib.rs` (`pub mod animations;`)
- Modify: `src/crates/concat-export/src/flatten.rs` (append the synthesized links)
- Modify: `src/crates/concat-host/src/titles.rs` (same)
- Modify: `src/crates/concat/ui/inspector/animations-panel.slint` (the Effects shelf)
- Modify: `src/crates/concat/src/studio.rs`, `src/crates/concat/src/lib.rs`, `ui/editor.slint`, `ui/workspace/config-pane.slint`, `ui/workspace/seat.slint` (the Effects models)
- Modify: `src/crates/concat/locales/*.json` (8 keys × 14 files)
- Modify: `CHANGELOG.md`
- Test: `src/crates/concat-export/src/animations.rs`, `src/crates/concat-host/tests/export.rs`, `motion.rs`

**Interfaces:**
- Consumes:
  - from Task 2: `Preset.effect: Option<EffectRamp>` and `Curve::{Still, FadeFast}`
  - from Task 3: `export_animations`
- Produces:
  - `concat_export::animations::animation_effects(clip: &model::Clip, source_start: f64, speed: Option<f64>) -> Vec<AppliedFilter>`, where `speed` of `None` means the clip's source map is not a straight line, so the links go unspanned
  - effect package `concat.glitch`, whose param is `amount`, 0–100

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src/crates/concat-core/src/motion.rs`:

```rust
    #[test]
    fn every_effect_preset_ramps_a_shipped_parameter_and_pairs_with_its_other_end() {
        let effects: Vec<&Preset> = PRESETS
            .iter()
            .filter(|preset| preset.group == Group::Effects)
            .collect();
        assert_eq!(effects.len(), 12);
        for preset in &effects {
            let ramp = preset.effect.expect("an Effects preset ramps an effect");
            assert!(ramp.effect.starts_with("concat."), "{}", preset.id);
            let twin = if let Some(stem) = preset.id.strip_suffix("-in") {
                format!("{stem}-out")
            } else {
                format!("{}-in", preset.id.strip_suffix("-out").expect("-in or -out"))
            };
            let twin = super::preset(&twin).expect("both ends");
            assert_eq!(twin.effect, preset.effect, "{}", preset.id);
            assert_ne!(twin.side, preset.side);
        }
    }
```

Create `src/crates/concat-export/src/animations.rs` with only tests:

```rust
// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

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
        assert!(animation_effects(&clip_with(Some("zoom-in"), Some("fade-out")), 10.0, Some(1.0)).is_empty());
    }
}
```

Add `pub mod animations;` to `src/crates/concat-export/src/lib.rs`, next to its other `pub mod` lines.

Add to `src/crates/concat-host/tests/export.rs`, after `an_animation_reaches_the_picture`:

```rust
/// An effect preset reaches the picture: quadrants that glitch in over a
/// second are torn at their first frame and whole by a second and a half.
#[test]
fn an_effect_animation_reaches_the_picture() {
    use concat_project::commands::AnimationSlot;
    use concat_project::model::ClipAnimation;

    let scratch = Scratch::new("effect-animations");
    let path = scratch.path().join("quadrants.mp4");
    quadrants(&path);
    let mut studio = Studio::new(scratch.path(), "Glitch", video(WIDTH, HEIGHT, 30, 1));
    let media = studio.import(&path);
    let clip = studio
        .apply(Command::AddClipAtFirstFree {
            media_id: media,
            start: 0.0,
        })
        .expect("placed");
    let still = studio.export("no animation");
    studio.apply(Command::SetClipAnimation {
        clip_id: clip,
        slot: AnimationSlot::In,
        animation: Some(ClipAnimation {
            id: "glitch-in".to_owned(),
            duration: 1.0,
        }),
    });
    let glitched = studio.export("glitch in");
    let differs = |a: &Frame, b: &Frame| {
        a.pixels()
            .chunks(4)
            .zip(b.pixels().chunks(4))
            .filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > 40))
            .count()
    };
    let pixels = (WIDTH * HEIGHT) as usize;
    assert!(
        differs(&still.frames[0], &glitched.frames[0]) > pixels / 100,
        "the first frame is torn"
    );
    assert!(
        differs(&still.frames[45], &glitched.frames[45]) < pixels / 200,
        "the frame at 1.5 s is whole"
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `wcargo test -p concat-core --lib motion 2>&1 | tail -10`
Expected: FAIL with `left: 0, right: 12` in `every_effect_preset_…`.

Run: `wcargo test -p concat-export --lib animations 2>&1 | tail -10`
Expected: a compile error, because `animation_effects` is not found.

- [ ] **Step 3: The glitch package**

`src/crates/concat-effects/packages/concat.glitch/effect.toml`:

```toml
format = 2

[effect]
id = "concat.glitch"
name = "Glitch"
version = 1
kind = "effect"
category = "Distort"
order = 55
intensity = "amount"
description = "Slices of the picture torn sideways and its colours pulled apart, like a broken digital signal."

[[param]]
key = "amount"
label = "Amount"
min = 0
max = 100
default = 60
step = 1
unit = "%"

[wgsl]
entry = "effect.wgsl"
```

`src/crates/concat-effects/packages/concat.glitch/effect.wgsl`:

```wgsl
struct Params { amount: f32 }

// A broken digital signal: the picture cut into horizontal slices, some of
// them torn sideways by their own amount, with red and blue pulled apart
// along the tear. The slices re-deal twelve times a second, so it flickers
// the way a bad feed does; at zero amount the picture comes through whole.
fn effect(uv: vec2<f32>) -> vec4<f32> {
    let k = params.amount * 0.01;
    let slice = floor(uv.y * 24.0);
    let tick = floor(frame.time * 12.0);
    let torn = step(0.55, hash(vec2<f32>(slice, tick), 2.0));
    let disp = (hash(vec2<f32>(slice, tick), 1.0) - 0.5) * 0.12 * k * torn;
    let split = 0.012 * k;
    let at = uv + vec2<f32>(disp, 0.0);
    let c = sample(at);
    let r = sample(at + vec2<f32>(split, 0.0)).r;
    let b = sample(at - vec2<f32>(split, 0.0)).b;
    return vec4<f32>(r, c.g, b, c.a);
}
```

`src/crates/concat-effects/packages/concat.glitch/fixtures.toml`:

```toml
# A flat picture has no edge to tear: what these pin is that the light past
# white and the alpha come through as they went in, however hard it tears.
# The tear itself is the card's to show.

[[probe]]
name = "default, a grey"
input = [0.18, 0.18, 0.18, 1]
expect = [0.18, 0.18, 0.18, 1]

[[probe]]
name = "default, half transparent"
input = [0.5, 0.2, 0.1, 0.5]
expect = [0.5, 0.2, 0.1, 0.5]

[[probe]]
name = "minimum, a highlight past white"
at = "min"
input = [4, 1, 0.05, 1]
expect = [4, 1, 0.05, 1]

[[probe]]
name = "maximum, a highlight past white"
at = "max"
input = [4, 1, 0.05, 1]
expect = [4, 1, 0.05, 1]
```

`hash(p: vec2<f32>, seed: f32) -> f32` and `sample` come from the effect head in `concat-effects/src/shader.rs` (lines 99–200 and 520). Film grain uses them the same way.

- [ ] **Step 4: The twelve Effects presets**

In `src/crates/concat-core/src/motion.rs`, remove any `#[allow(dead_code)]` Task 2 put on `Curve::Still` and `Curve::FadeFast`. Add a constructor after `basic`:

```rust
const fn effect(
    id: &'static str,
    side: Side,
    label: &'static str,
    curve: Curve,
    ramp: EffectRamp,
) -> Preset {
    Preset {
        id,
        side,
        group: Group::Effects,
        label,
        curve,
        effect: Some(ramp),
    }
}

const GLITCH: EffectRamp = EffectRamp { effect: "concat.glitch", param: "amount", from: 100.0, rest: 0.0 };
const BLUR: EffectRamp = EffectRamp { effect: "concat.gaussian-blur", param: "radius", from: 40.0, rest: 1.0 };
const PIXELATE: EffectRamp = EffectRamp { effect: "concat.pixelate", param: "size", from: 64.0, rest: 2.0 };
const RGB_SPLIT: EffectRamp = EffectRamp { effect: "concat.chromatic-aberration", param: "shift", from: 24.0, rest: 1.0 };
const FLASH: EffectRamp = EffectRamp { effect: "concat.exposure", param: "stops", from: 3.0, rest: 0.0 };
const ZOOM_BLUR: EffectRamp = EffectRamp { effect: "concat.zoom-blur", param: "amount", from: 100.0, rest: 0.0 };
```

Append these entries to the end of `PRESETS`:

```rust
    effect("glitch-in", Side::In, "animations.preset.glitch", Curve::Still, GLITCH),
    effect("blur-in", Side::In, "animations.preset.blur", Curve::FadeFast, BLUR),
    effect("pixelate-in", Side::In, "animations.preset.pixelate", Curve::Still, PIXELATE),
    effect("rgb-split-in", Side::In, "animations.preset.rgbSplit", Curve::Still, RGB_SPLIT),
    effect("flash-in", Side::In, "animations.preset.flash", Curve::Still, FLASH),
    effect("zoom-blur-in", Side::In, "animations.preset.zoomBlur", Curve::Zoom(0.6), ZOOM_BLUR),
    effect("glitch-out", Side::Out, "animations.preset.glitch", Curve::Still, GLITCH),
    effect("blur-out", Side::Out, "animations.preset.blur", Curve::FadeFast, BLUR),
    effect("pixelate-out", Side::Out, "animations.preset.pixelate", Curve::Still, PIXELATE),
    effect("rgb-split-out", Side::Out, "animations.preset.rgbSplit", Curve::Still, RGB_SPLIT),
    effect("flash-out", Side::Out, "animations.preset.flash", Curve::Still, FLASH),
    effect("zoom-blur-out", Side::Out, "animations.preset.zoomBlur", Curve::Zoom(0.6), ZOOM_BLUR),
```

Before relying on these parameters, check each key and range in its package's `effect.toml`:
- `concat.gaussian-blur`: `radius`, 1–50
- `concat.pixelate`: `size`, 2–64
- `concat.chromatic-aberration`: `shift`, 1–24
- `concat.exposure`: `stops`, −4 to 4
- `concat.zoom-blur`: `amount`, 0–100

A `from` outside the range is a plan defect: clamp it into the range and record a ruling in the ledger.

- [ ] **Step 5: `animation_effects`**

Put this above the tests in `src/crates/concat-export/src/animations.rs`:

```rust
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
        out.push(link(ramp, [(0.0, ramp.from), (end, ramp.rest)], speed.map(|speed| Span {
            from: source_start,
            to: source_start + played.seconds * speed,
        })));
    }
    if let Some(played) = exit
        && let Some(ramp) = motion::preset(&played.id).and_then(|preset| preset.effect)
    {
        let begin = (length - played.seconds) / length;
        // One second past the end: the span's `to` is itself outside, and
        // the last frame must still be inside.
        out.push(link(ramp, [(begin, ramp.rest), (1.0, ramp.from)], speed.map(|speed| Span {
            from: source_start + (length - played.seconds) * speed,
            to: source_start + length * speed + 1.0,
        })));
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
                ease: KeyEase([0.0, 0.0, 1.0, 1.0]),
            })
            .collect(),
    )]);
    link.span = span;
    link
}
```

`KeyEase([0.0, 0.0, 1.0, 1.0])` is the linear bezier. If `concat_project::model` names a linear constant (`KeyEase::LINEAR`), use it. The test in `an_out_ramps_up_into_the_last_frame` expects `span.to == 14.0 + 1.0`, which matches the `+ 1.0` above.

- [ ] **Step 6: Append the links where clips are flattened**

In `src/crates/concat-export/src/flatten.rs`, in the media clip's `ExportClip` literal, replace `effects: clip.video_effects.clone(),` with:

```rust
                effects: {
                    let speed = match clip.kind {
                        ModelClipKind::Image => Some(1.0),
                        _ if clip.speed_curve.is_some() => None,
                        _ => Some(clip.speed),
                    };
                    let mut chain = clip.video_effects.clone();
                    chain.extend(crate::animations::animation_effects(
                        clip,
                        clip.source_start,
                        speed,
                    ));
                    chain
                },
```

In `src/crates/concat-host/src/titles.rs`, replace `effects: clip.video_effects.clone(),` in the title literal with:

```rust
                    effects: {
                        let mut chain = clip.video_effects.clone();
                        chain.extend(concat_export::animations::animation_effects(
                            clip,
                            0.0,
                            Some(1.0),
                        ));
                        chain
                    },
```

A title renders as a still whose source clock starts at zero, with a rate of one.

- [ ] **Step 7: The Effects shelf and its strings**

Each shelf gets its own model, so a grid only ever lays out its own cards and leaves no gaps.

In `src/crates/concat/src/studio.rs`:
- Add `animation_effects_in` and `animation_effects_out`, both `Rc<VecModel<AnimationPresetData>>`, to `Models` beside the Basic pair, initialised the same way.
- In `fill_animation_presets`, fill them in the same loop, from `animations::offered(slot)` filtered to `Group::Effects`. Loop over `(slot, basic_model, effects_model)` triples and share the `.map(…)`. Update the doc comment to say it publishes both shelves of each end.

In `src/crates/concat/src/lib.rs`, next to the two `set_animation_presets_*` lines:

```rust
        editor.set_animation_effects_in(ModelRc::from(models.animation_effects_in.clone()));
        editor.set_animation_effects_out(ModelRc::from(models.animation_effects_out.clone()));
```

Carry the two models through the UI:
- In `editor.slint`, add `in-out property <[AnimationPresetData]> animation-effects-in;` and `…-out;`.
- In `config-pane.slint`, add `in property <[AnimationPresetData]> animation-effects-in;` and `…-out;`, and pass them to the panel as `effects-in: root.animation-effects-in; effects-out: root.animation-effects-out;`.
- In `seat.slint`, add `animation-effects-in: Editor.animation-effects-in;` and `animation-effects-out: Editor.animation-effects-out;`.

In `src/crates/concat/ui/inspector/animations-panel.slint`, add to `AnimationsPanel`:

```slint
    in property <[AnimationPresetData]> effects-in;
    in property <[AnimationPresetData]> effects-out;
    property <[AnimationPresetData]> effects: root.out ? root.effects-out : root.effects-in;
```

and replace the line `// Task 6 adds the Effects shelf here.` with:

```slint
    Subhead { title: I18n.t("common.effects"); }
    AnimationGrid {
        presets: root.effects;
        with-none: false;
        chosen: root.chosen;
        picked(id) => { root.pick(root.out, id); }
    }
```

Save as `.superpowers/sdd/2026-09-29-clip-animations/locales_task6.py` the same script as Task 5, with these `KEYS` and `T`:

```python
KEYS = ["animations.preset.glitch", "animations.preset.blur", "animations.preset.pixelate",
        "animations.preset.rgbSplit", "animations.preset.flash", "animations.preset.zoomBlur",
        "effects.glitch.name", "effects.glitch.description"]
T = {
 "en": ["Glitch","Blur","Pixelate","RGB split","Flash","Zoom blur","Glitch","Slices of the picture torn sideways and its colours pulled apart, like a broken digital signal."],
 "de": ["Glitch","Unschärfe","Verpixeln","RGB-Versatz","Blitz","Zoom-Unschärfe","Glitch","Bildstreifen seitlich verrissen und die Farben auseinandergezogen, wie ein gestörtes Digitalsignal."],
 "es": ["Glitch","Desenfoque","Pixelar","Separación RGB","Destello","Desenfoque de zoom","Glitch","Franjas de la imagen desgarradas hacia los lados y los colores separados, como una señal digital rota."],
 "fa": ["گلیچ","تاری","پیکسلی","جدایی RGB","فلاش","تاری زوم","گلیچ","برش‌هایی از تصویر که به پهلو کشیده شده‌اند و رنگ‌هایی که از هم جدا شده‌اند، مانند یک سیگنال دیجیتال خراب."],
 "fr": ["Glitch","Flou","Pixelisation","Décalage RVB","Flash","Flou de zoom","Glitch","Des tranches de l'image arrachées sur le côté et les couleurs décalées, comme un signal numérique abîmé."],
 "hr": ["Glitch","Zamućenje","Pikselizacija","RGB razdvajanje","Bljesak","Zamućenje zumiranjem","Glitch","Trake slike razderane u stranu i razdvojene boje, poput pokvarenog digitalnog signala."],
 "it": ["Glitch","Sfocatura","Pixel","Separazione RGB","Flash","Sfocatura zoom","Glitch","Strisce dell'immagine strappate di lato e colori separati, come un segnale digitale guasto."],
 "ja": ["グリッチ","ぼかし","モザイク","RGBずれ","フラッシュ","ズームぼかし","グリッチ","映像を横に引き裂き、色をずらす、壊れたデジタル信号のような効果。"],
 "ko": ["글리치","흐림","픽셀화","RGB 분리","플래시","줌 흐림","글리치","화면 조각이 옆으로 찢기고 색이 어긋나는, 고장 난 디지털 신호 같은 효과입니다."],
 "pt-BR": ["Glitch","Desfoque","Pixelizar","Separação RGB","Clarão","Desfoque de zoom","Glitch","Faixas da imagem rasgadas para o lado e as cores separadas, como um sinal digital com defeito."],
 "ru": ["Глитч","Размытие","Пикселизация","Разделение RGB","Вспышка","Размытие зума","Глитч","Полосы кадра, сорванные вбок, и разъехавшиеся цвета, как у сломанного цифрового сигнала."],
 "tr": ["Glitch","Bulanıklık","Pikselleştir","RGB ayrışması","Parlama","Zoom bulanıklığı","Glitch","Görüntünün şeritleri yana yırtılmış ve renkleri ayrılmış, bozuk bir dijital sinyal gibi."],
 "zh-Hans": ["故障","模糊","像素化","RGB 分离","闪光","缩放模糊","故障","画面被横向撕裂成条、颜色错位，像损坏的数字信号。"],
 "zh-TW": ["故障","模糊","像素化","RGB 分離","閃光","縮放模糊","故障","畫面被橫向撕裂成條、顏色錯位，像損壞的數位訊號。"],
}
```

Run: `python3 .superpowers/sdd/2026-09-29-clip-animations/locales_task6.py src/crates/concat/locales`

`every_package_and_preset_key_is_in_the_inventory` checks that `effects.glitch.name` and `effects.glitch.description` match the manifest's English exactly, and the strings above are copied from `effect.toml`. The same test may also want the param label (`effects.labels.amount`) and category (`effects.categories.distort`), but both already exist for other packages. If it names another missing key, add it to all 14 files the same way.

In `CHANGELOG.md`, under `## Unreleased`, add a `### Clips` heading, or add to it if it already exists:

```markdown
- **Animations.** A video, a still or a title can come in and go out with
  a preset: fade, zoom, slide, spin, pop, shake, and effect presets that
  glitch, blur, pixelate, split the colours, flash or zoom-blur over the
  first or last seconds. Pick one on the clip's new Animations tab, set
  its length, and the viewer plays it at once. A bar along each end of a
  clip on the timeline shows how long it animates. A new Glitch effect
  joins the Effects shelf.
```

- [ ] **Step 8: Run everything**

Run: `wcargo test -p concat-core -p concat-project -p concat-export -p concat-effects --lib 2>&1 | tail -20`
Expected: every test passes, including the effect package fixtures for `concat.glitch` in `concat-effects`.

Run: `wcargo test -p concat-host --test export an_effect_animation_reaches_the_picture an_animation_reaches_the_picture 2>&1 | tail -15`
Expected: both pass. If `an_effect_animation_reaches_the_picture` fails on the torn-frame count, the glitch is too weak at this size (160×90): raise `0.12` in `effect.wgsl` to `0.2`, never the test's threshold, and record a ruling in the ledger.

Run: `wcargo test -p concat --lib 2>&1 | tail -20`
Expected: every test passes, apart from the known flaky pair (see Task 5, step 8).

Run: `wcargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -5`
Expected: no warnings.

Run: `wcargo fmt --all`, then `wcargo fmt --all --check && echo FMT_CLEAN`
Expected: `FMT_CLEAN`.

- [ ] **Step 9: Commit, with the spec and the plan**

```bash
wgit add src CHANGELOG.md docs/superpowers/specs/2026-09-29-clip-animations-design.md docs/superpowers/plans/2026-09-29-clip-animations.md
# msg.txt: "feat(animations): effect presets and a Glitch effect" + body + trailer
wgit commit -F .superpowers/sdd/2026-09-29-clip-animations/msg.txt
```

---

## Where this plan departs from the spec

These are decided in the plan, so no ruling is needed at execution:

- **Replay on release becomes a button.** The spec says letting go of the length slider replays the animation, but `Param` has no release callback. The plan adds a replay button (`animations.replay`) beside the slider instead. Picking a card still plays immediately.
- **The effect links are added in `flatten.rs` and `titles.rs`, not `resolve.rs`.** Those are the two places where both preview and export build their clips, and both already copy `video_effects`.
- **Card art uses glyphs.** Motion cards wear an existing `Glyph` rather than a new drawing. Effect cards reuse the effect's still, as the spec says.
- **Glitch goes on the "Distort" shelf.** The existing single-picture distortion effects are there. The spec's "Glitch" shelf is used by transitions.
