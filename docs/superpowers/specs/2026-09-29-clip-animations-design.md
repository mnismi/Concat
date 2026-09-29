# Clip animations (In and Out): design

Date: 2026-09-29 · Branch: `feat/clip-animations`

## Goal

Every item on screen, whether a video, a still or a title, can be given an
**In** animation that plays over its first seconds and an **Out** animation
that plays over its last seconds. You pick each from a set of presets (fade,
zoom, slide, spin, pop, glitch, blur and so on) and set its length. Like
CapCut's clip animations.

What the user asked for:

- A new **Animations** tab in the clip inspector, beside the tabs a clip
  already has.
- **In** and **Out**, each with its own preset and its own length.
- Presets of two kinds: **motion** (zoom in, zoom out, slide...) and
  **effect** (glitch and similar), both in this feature.
- Works on **text too**. Transitions sit on the cut between two clips, but
  an animation belongs to one item, so a title alone on its track gets one.
- **In and Out only.** CapCut's looping "combo" animations and per-letter
  text animations are later features.

## The model (`concat-project`)

```rust
/// A preset played over one end of a clip.
pub struct ClipAnimation {
    /// Which preset, e.g. "zoom-in". Stable forever: project files store it.
    pub id: String,
    /// Seconds of timeline it covers, measured from the clip's end it sits on.
    pub duration: f64,
}

// On `Clip`, beside `transition_in`:
#[serde(default, skip_serializing_if = "Option::is_none")]
pub animation_in: Option<ClipAnimation>,
#[serde(default, skip_serializing_if = "Option::is_none")]
pub animation_out: Option<ClipAnimation>,
```

- Optional and skipped when empty, so older projects load unchanged and a
  project without animations writes the same bytes as before.
- An id this build does not know is kept in the file and drawn as no
  animation, the same way an unknown effect id is skipped at render time.
- **Kinds:** video, image and text. Audio clips and effect layers have
  nothing to move, and the command refuses them.

### The command

```rust
SetClipAnimation {
    clip_id: String,
    slot: AnimationSlot,               // In | Out
    animation: Option<ClipAnimation>,  // None removes it
}
```

- One undo step. Dragging the length slider is one step too, sent on release
  the way the other sliders are.
- Clamping:
  - `duration` is kept within 0.1 s and the clip's length.
  - When In plus Out would be longer than the clip, the *other* side is
    shortened to fit. The side you are editing keeps the value you set.
- Refusals, as new `CommandError` variants: the clip is gone, or its kind
  cannot animate.
- The API and the command line pass `Command` through, so it is scriptable
  as soon as it exists.

### Edits that change a clip

- **Split** (and silence removal, which uses the same code path): the In
  animation stays on the first piece and the Out on the last, exactly like
  `fade_in` and `fade_out` (see `keep_the_ends` in `commands/cut.rs`).
- **Trim and speed changes:** the lengths are in seconds and stay as set.
  They are clamped again when the clip gets shorter than In plus Out.
- **Duplicate and copy-paste** carry both slots with the clip.

## Drawing it (`concat-core/src/motion.rs`)

This is a pure module with no files and no GPU. Preview and export both
build the frame plan through `concat-export` onto `concat_core::timeline::Clip`,
so the one definition is what every renderer draws.

```rust
/// What a preset does to a clip at one instant, relative to the clip's own.
pub struct Motion {
    pub scale: f64,     // factor, 1 = as placed
    pub offset_x: f64,  // added, in frame widths
    pub offset_y: f64,  // added, in frame heights
    pub rotation: f64,  // added, degrees
    pub opacity: f64,   // factor, 1 = as set
}

/// The preset at `progress` through it: 0 is the start of its window and 1
/// the end. An In animation plays 0→1 into the clip's resting state; an Out
/// animation plays the same curve from 1 back to 0 into its last frame.
pub fn motion_at(preset: &str, progress: f64) -> Motion;
```

- **Engine clip:** `concat_core::timeline::Clip` gains
  `entrance: Option<Played>` and `exit: Option<Played>`. `Played` holds the
  preset id and its window in seconds. `transform_at` and `opacity_at` apply
  the clip's keyframes first and the preset's `Motion` on top: scale and
  opacity multiply, offsets and rotation add. A clip you keyed yourself
  still animates in and out.
- **Outside the window** `Motion` is the identity, so a clip with no
  animation costs one comparison per frame.
- **Easing:** each preset has its own curve, built from `animate::Ease`
  (ease-out for In, ease-in for Out), so "pop" can overshoot and "fade" is
  gentle.

### Motion presets (step 1)

| In | Out | What it does |
|---|---|---|
| Fade in | Fade out | opacity 0 → 1 |
| Zoom in | Zoom out | scale 0.6 → 1 with opacity 0 → 1 |
| Zoom from big | Zoom to big | scale 1.6 → 1 with opacity 0 → 1 |
| Slide from left, right, top, bottom | Slide to left, right, top, bottom | offset 0.3 of the frame → 0 with opacity 0 → 1 |
| Rise | Sink | offset_y +0.08 → 0 with opacity 0 → 1 |
| Spin in | Spin out | rotation −180° → 0 and scale 0.3 → 1 with opacity |
| Pop | Pop out | scale 0 → 1.1 → 1 (overshoot) with opacity |
| Shake | Shake out | a decaying horizontal wobble; opacity rises over the first quarter |

The Out presets use the In curve played backwards (progress runs 1 → 0 over
the window), so an Out window ends on the clip's last frame fully gone.

### Effect presets (step 2)

These ramp a video effect over the window. The resolver in
`concat-export/src/resolve.rs` adds a **synthesized** effect link to the
clip's chain for the frame plan only, with the effect's strength keyed over
the window. It is never written into `video_effects`, so it never shows in
the Effects tab and can't be edited there.

| In / Out | Effect used | Parameter ramp (In; Out is reversed) |
|---|---|---|
| Glitch | new `concat.glitch` (below) | amount 100 → 0 |
| Blur in / Blur out | `concat.gaussian-blur` | radius 40 → 1, with opacity 0 → 1 over the first half |
| Pixelate | `concat.pixelate` | size 64 → 2 |
| RGB split | `concat.chromatic-aberration` | shift 24 → 1 |
| Flash | `concat.exposure` | stops +3 → 0 |
| Zoom blur | `concat.zoom-blur` | amount 100 → 0, with the zoom-in motion |

- **The effect is gone outside the window.** The synthesized link is limited
  to the window by the link `span` already in the model.
- **A new effect package, `concat.glitch`** (kind `effect`, category
  `Glitch`): a single-input slice tear plus RGB split with one `amount`
  parameter, adapted from `concat.glitch-warp`'s shader. The glitch packages
  there today are transitions, which need two pictures. It also appears on
  the Effects shelf as an ordinary effect.
- **Pairing with motion:** an effect preset can carry a `Motion` too (Blur in
  also fades, Zoom blur also zooms). It is one preset id with both parts, not
  two slots.

## The Animations tab

- **Tab strip** (`ui/workspace/config-pane.slint`):
  - Video: Video · Audio · Effects · **Animations**
  - Image: Video · Effects · **Animations**
  - Text: Text · Effects · **Animations**
  - Audio and effect layers are unchanged. Video goes to four tabs, over the
    comment's "three at most" rule, and the comment is updated to say why.
    If the strip is too tight at the narrowest panel width, labels shorten
    rather than wrap.
- **In | Out switch** at the top, in the segmented style of the section
  strip. Each side shows its own choice.
- **Preset grid**, cards like the Effects shelf, in two groups: **Basic**
  (motion) and **Effects**.
  - The first card is **None**, which removes the animation.
  - The chosen card is highlighted.
  - Card art: motion presets get a small drawn glyph of their movement, and
    effect presets reuse their effect's still.
- **Length slider** under the grid, shown only when a preset is set. It runs
  from 0.1 s to the clip's length minus the other side's length, and
  defaults to 0.5 s (or half the clip, when that is shorter).
- **Preview on pick:** clicking a card applies it, then plays just that
  window in the viewer (the first N seconds for In, the last N for Out) and
  stops. Letting go of the length slider replays it.
- **On the timeline** (`ui/timeline/lanes.slint`): an animated clip shows a
  thin bar at its start or end, as wide as the window, drawn like the fade
  handles.
- **Window side:** a new `concat/src/panes/animations.rs` holds the preset
  list shown in the grid, the `Msg` and the update. The same preset table
  from `concat-core::motion` names every id, so the grid and the renderer
  cannot drift apart.
- **Strings:** new keys in `concat/locales/en.json` and in all 13 other
  shipped locales. The locale test requires full coverage, and the other
  languages are machine-translated for a native speaker to check.

## Testing

- **Motion** (unit, `motion.rs`):
  - For every preset: the identity at the resting end, the stated start
    values at the other, and values within range between.
  - Pop overshoots above 1.
  - Out is In reversed.
  - An unknown id is the identity.
- **Engine clip** (unit, `concat-core`):
  - `transform_at` and `opacity_at` inside and outside both windows.
  - Composing with user keys (a keyed scale times zoom-in).
  - In and Out on one clip.
- **Command** (unit, `concat-project`):
  - Set, replace and remove.
  - Clamping to the clip, and the other side shortening to fit.
  - The refusal for audio and layer clips.
  - A split keeps In on the head and Out on the tail.
  - Silence removal keeps them on the first and last kept pieces.
  - Duplicate carries them.
  - One undo restores the project.
  - An old project with no fields round-trips byte for byte.
  - An unknown preset id round-trips.
- **Resolve** (unit, `concat-export`):
  - An effect preset adds one synthesized link, spanned to the window with
    keyed strength.
  - `video_effects` in the project is untouched.
- **End to end** (`concat-host/tests/export.rs`): a still with Fade in 1 s
  exports with a black first frame and a bright frame at 1.5 s. A still with
  Glitch in exports with its first frame different from its frame at 1.5 s.
- **By hand**, in the app:
  - Each preset on a video, a still and a title.
  - Preview on pick.
  - Trimming an animated clip.

## Built in two steps

1. **Motion:**
   - the model and command
   - `motion.rs` and the engine clip
   - the split and cut handling
   - the Animations tab with the Basic group
   - the timeline bar
   - the strings
2. **Effects:**
   - the `concat.glitch` package
   - the synthesized effect links in resolve
   - the Effects group in the tab

The feature is done when both steps are in.

## Not in this version

- Looping "combo" animations.
- Per-letter or per-word text animations (typewriter and similar).
- The phone layout's editor.
- Applying a preset to several selected clips at once.
- Dragging an animation from a shelf onto a clip.
- A curve editor for a preset's easing.
