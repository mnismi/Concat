# Pieces (reusable saved elements): design

Date: 2026-09-29 · Branch: `feat/pieces`, off `feat/clip-animations`

## Goal

Save a few clips as a reusable **piece**, such as a hand that swoops in and
points with a whoosh, or a styled title with its own In/Out animation. Then
drop that piece into any later project as one sealed unit.

What the user asked for:

- Save something once, reuse it in future projects.
- **One unit** on the timeline. The hand and its sound move together and the
  inside of the piece is locked.
- Editable after dropping it in: **position, rotation and scale** of the
  whole piece, and **the words** of any text inside it. Nothing else.
- Dragging a piece longer **holds its still middle**. The in-motion and
  out-motion keep their saved speed.

How this differs from templates: a template (`concat-host/src/templates.rs`)
is a whole project with slots, and using one creates a new project. A piece
is a few clips that you insert into the project already open. Pieces reuse
the template bundle format and its file-copying code.

## Terms

- **Piece:** the sealed mini-edit. It has a name, a design frame, a length, a
  hold window, and inner clips on inner lanes.
- **Piece clip:** a timeline clip of kind `Piece` that points at a piece and
  carries only the user's edits.
- **Design frame:** the width and height of the timeline the piece was saved
  from.
- **Hold window:** `[hold_in, hold_out]` in piece seconds. This is the still
  middle that stretches. It is empty when the piece is fixed length.
- **Expansion:** replacing piece clips with their inner clips, transformed
  and stretched. It is used for rendering only and is never saved.

## The model (`concat-project`)

```rust
/// A sealed mini-edit, stored once per project however often it is placed.
pub struct Piece {
    /// Minted by the editor ("p1", "p2", ...).
    pub id: String,
    pub name: String,
    /// The frame it was designed in; placement fits it into the timeline's.
    pub design_width: u32,
    pub design_height: u32,
    /// Saved length in seconds.
    pub duration: f64,
    /// The still middle, in piece seconds. `None` is a fixed-length piece.
    pub hold: Option<Hold>,
    /// Inner lanes, top to bottom, each an ordered list of ordinary clips
    /// whose `start` is piece seconds. Never contains a `Piece` clip.
    /// Inner clip ids are the piece's own namespace; `track_id` is empty.
    pub lanes: Vec<PieceLane>, // PieceLane { clips: Vec<Clip>, extra }
    /// Fields this build does not know, kept so they round-trip.
    pub extra: Map<String, Value>,
}

// On `Project`, beside `media`:
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub pieces: Vec<Piece>,

// New `ClipKind::Piece`. On `Clip`:
/// For a `Piece` clip: which piece, and the user's words for each inner
/// text clip, keyed by inner clip id. A missing key keeps the saved words.
#[serde(default, skip_serializing_if = "Option::is_none")]
pub piece: Option<PiecePlacement>,

pub struct PiecePlacement {
    pub piece_id: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub texts: BTreeMap<String, String>,
}

// On `MediaItem`:
/// True when the item came in with a piece. The media bin hides it.
#[serde(default, skip_serializing_if = "is_false")]
pub piece_media: bool,
```

- A piece clip's **position, rotation and scale** are the clip's existing
  `offset_x`, `offset_y`, `rotation` and `scale`, and its **length** is
  `duration`. The preview's transform handles and the existing transform
  commands work on it unchanged. Its keys, speed, effects, filters,
  animations and volume are not used; commands that set those refuse a
  `Piece` clip.
- Every new field is optional and skipped when empty, so older projects load
  unchanged and a project with no pieces writes the same bytes as before.
- The ten placements of one piece all share one `Piece` entry. When the
  project is saved, a piece that no clip on any timeline points at is
  dropped, and so is `piece_media` that no remaining piece or clip uses.
- A piece clip whose piece is missing renders nothing. It is not a load
  error, and the clip keeps its place so the piece can come back.
- A build from before pieces reads a piece clip as a video with no media
  and drops it. That is the reader's existing tolerance, and pieces make no
  attempt to degrade further.

### Commands

All of these are one undo step each.

```rust
/// Adds the piece, its media and its fonts to the project if they are not
/// there yet, then places one piece clip at `start`: on `track_id`, or on
/// the first free lane when None.
InsertPiece {
    piece: Piece,
    media: Vec<MediaItem>,
    fonts: Vec<CustomFont>,
    track_id: Option<String>,
    start: f64,
},
/// Places one more clip of a piece the project already holds. What
/// duplicate and paste use.
PlacePiece { piece_id: String, track_id: Option<String>, start: f64 },
/// Sets or clears (None) one inner text clip's words on a placement.
SetPieceText { clip_id: String, inner_clip_id: String, text: Option<String> },
/// Replaces a piece clip with its expanded clips, as ordinary clips.
UnpackPiece { clip_id: String },
```

- **Media and fonts** are matched by path. An item already in the bin is
  reused, and a new one is minted an "m" id and marked `piece_media`. The
  piece's inner clips are rewritten to the project's media ids.
- **Pieces** are matched by content once their media ids are rewritten:
  dropping the same library piece twice reuses the first entry. A new piece
  is minted a "p" id.
- Inner clip ids stay the piece's own. Expansion prefixes them with the
  piece clip's id, so they never collide with the timeline's clips.
- **Trimming** a piece clip goes through the existing trim command. A tail
  trim is clamped to the limits under *Hold stretch*. A head trim does
  nothing; the piece is moved to move its start. A fixed-length piece does
  not trim at all.
- Every command that would change a piece clip's inside is refused with
  `CommandError::PieceIsSealed`: keys, effect keys, speed, speed curve,
  cutout, animations, freeze frame, media replacement, merge, silence
  removal, and any clip patch other than a rename. Split skips piece clips.
  Move, delete, ripple-delete, lane changes and `SetClipTransform` (scale,
  offset and rotation) work as on any other clip.

## Expansion

```rust
/// `project` with every `Piece` clip on `timeline_id` (the active one when
/// None) replaced by its inner clips, transformed and stretched. Borrowed
/// unchanged when there are no piece clips. Render-only; never saved.
pub fn expand_pieces(project: &Project, timeline_id: Option<&str>) -> Cow<'_, Project>
```

- It lives in `concat-project` (`pieces.rs`) and is a pure function over the
  model.
- Preview and export all go through
  `concat_export::flatten::flatten_timeline_in`, and every title goes
  through `concat_host::titles::Titles::clips_with`. Both expand on entry,
  so every caller gets pieces without knowing about them.
- An expanded clip's id is `"{piece clip id}/{inner clip id}"`.
- `UnpackPiece` is expansion of one clip, committed.

### Lanes

A piece clip on lane *L* expands into new lanes inserted just above *L*
(before it in the top-to-bottom list), keeping the piece's inner
top-to-bottom order. Their ids are `"{piece clip id}/L{n}"`. The piece
clip's own lane keeps its other clips. Hidden or muted on *L* applies to
every inner lane it expands into.

### Transform

Design frame `(dw, dh)`, timeline frame `(W, H)`, fit `f = min(W/dw, H/dh)`.
The piece clip supplies offset `(px, py)`, rotation `θ` and scale `S`.

For each inner visual clip, a point `c` in design pixels, measured from the
design frame's centre, maps to

```
c' = (W/2 + px·W, H/2 + py·H) + R(θ) · (S · f · c)
```

- **Offset:** the inner clip's centre maps through that formula and is
  written back as a fraction of `(W, H)`.
- **Rotation:** `rotation + θ`.
- **Scale:** chosen so that the picture's pixel size is `S · f` times its
  size in the design frame. The design fit and the target fit of that media
  differ by the media's aspect, and the new scale corrects for it.
  `stretch_x` and `stretch_y` carry over unchanged.
- **Text:** `font_size` (a fraction of frame height) becomes
  `font_size · S · f · dh / H`, and any box sizes scale the same way.
- **Keys:** keyed scale maps by the same factor as the constant, and keyed
  rotation by `+ θ`; both are exact. Offsets are the pair `(x, y)`:
  - With `θ = 0`, x maps from x alone and y from y alone, key by key, which
    is exact.
  - With `θ ≠ 0`, where only one of the pair is keyed, or both are keyed at
    the same instants with the same eases, both come out keyed at those
    instants with those eases. This is exact, because eased interpolation
    commutes with an affine map.
  - Otherwise the pair is resampled every 1/30 of a second across the
    clip, as linear keys.
- **In/Out motion presets** play as they are: their slides move in frame
  directions and their distances are fractions of the frame. Inside a
  rotated piece, a slide-left still slides left on screen. Baking the
  motion into keys would need the export crate's effect synthesis inside
  the engine; it is left for later if it ever matters.
- Audio clips take no transform.

### Hold stretch

The hold window is computed once, when the piece is saved, from its visual
inner clips (video, image, text, layer), in piece seconds:

A clip's **marks** are the piece times of:
- its keys
- its effects' keys
- the ends of its effects' spans (source seconds, converted at the clip's
  speed)

Then:
- **Entry end** of a clip is the latest of:
  - its `start`
  - `start + animation_in.duration`
  - its marks in its first half
- **Exit start** of a clip is the earliest of:
  - its end
  - `end − animation_out.duration`
  - its marks in its second half
- `hold_in = max(entry end)` and `hold_out = min(exit start)` over all visual
  clips.
- If `hold_in ≥ hold_out`, the piece is fixed length. It is also fixed length
  if any clip that spans the hold has a speed curve.

By construction, no visual clip starts, ends or has a key inside the hold,
so stretching by `Δ = placed duration − piece duration` is unambiguous:

- A clip ending at or before `hold_in` is unchanged.
- A clip starting at or after `hold_out` shifts later by `Δ`.
- A visual clip spanning the hold grows by `Δ`. Its marks are remapped as
  absolute piece times: marks at or before `hold_in` stay, and marks at or
  after `hold_out` shift by `Δ`. Keys are then re-expressed as `at` over the
  new length, and span ends as source seconds. In/Out animation lengths are
  in seconds, so they need no change.
- **Audio never stretches.** A sound that starts before `hold_out` keeps its
  place and length. A sound that starts after it shifts by `Δ`.
- **Shortest length:** `duration − (hold_out − hold_in)`, which squeezes the
  hold to nothing.
- **Longest length:** unlimited, except for spanning video clips. Each one
  can grow only by the source it has left,
  `(media duration − source_start) / speed − duration`, and the smallest of
  those is the limit.

### Text

For each `texts` entry, the inner text clip with that id takes those words as
its `TextStyle::content`. Its font, colour, layout and animation are
unchanged. An entry naming an inner clip that is not there is ignored.

## The library (`concat-host`)

`concat-host/src/pieces.rs` mirrors `templates.rs`:

```
<config>/pieces/<name>/
  piece.json   – { "concat", "version", "piece": Piece, "media": [...] }
  assets/      – every file the piece uses, fonts included
  poster.jpg   – one frame from the middle of the hold (or of the piece)
```

- `save(config, project, timeline_id, clip_ids, name, project_dir)`:
  - Builds the `Piece` from the selection. The earliest start becomes zero,
    each track the selection touches becomes an inner lane in lane order,
    the design frame is the timeline's, and the hold is computed.
  - A selected piece clip is expanded into the selection first, so a saved
    piece never nests.
  - Copies every file into `assets/`. `bundle_file` moves out of
    `templates.rs` into a shared `bundle.rs` so both use one copy.
- `list(config)`, `rename(config, path, name)`, `delete(config, path)`
  (guarded exactly as templates' delete is), `load(path)`. `load` returns
  the `Piece` and its media with `assets/` paths resolved absolute, ready
  for `InsertPiece`.
- Inserting copies the piece's assets into the project's `assets/` first, so
  the project stands alone, as instantiating a template does.
- **Refused, with a message the user sees:**
  - an empty selection
  - a template slot (placeholder) in the selection
  - a clip whose file is missing
  - a name already in the library (never overwritten)
- The poster is best effort, as with templates. A piece without one shows a
  glyph card.

## The window (`concat`)

- **Saving:** the timeline's clip context menu gains **Save as piece…**. It
  asks for a name and saves the current selection.
- **Shelf:** the media pane gets a **Pieces** section beside Templates, a
  card grid of posters and names. A card's context menu has **Rename** and
  **Delete**; Delete asks for confirmation.
- **Placing:**
  - Drag a card onto a lane at a time.
  - Double-click a card to place it at the playhead on the selected lane.
  - It lands at the saved length, centred, at scale 1.
- **Timeline bar:** shows the piece's name and a piece glyph. A fixed-length
  piece shows no trim handles.
- **Inspector** for a selected piece clip:
  - Position, Rotation and Scale controls, using the same controls ordinary
    clips have.
  - One text box per inner text clip, labelled with that clip's name, in
    lane order.
  - Nothing else.
- **Unpack** in the piece clip's context menu.
- New strings go through `I18n` like every other label.

## Testing

- **`expand_pieces` unit tests** (`concat-project`):
  - Transform: offset, rotation and scale composition, the fit between
    16:9 and 9:16 frames, and text size scaling.
  - Keys map exactly when x and y line up, and are resampled when they do
    not and `θ ≠ 0`.
  - In/Out motion is baked into keys under rotation.
  - Stretch: clips before, spanning and after the hold; sound kept and
    shifted; key remapping; shortest and longest length; fixed length.
  - Text replacement, including an unknown inner id being ignored.
  - Lane order, and hide/mute inheritance.
  - A missing piece renders nothing.
- **Hold computation:** from In/Out animations, from keys, from clip
  boundaries, and the fixed-length cases.
- **Commands:**
  - `InsertPiece` reuses an identical piece and re-mints ids.
  - `SetPieceText` and `UnpackPiece` each undo in one step.
  - Refused commands on a `Piece` clip.
  - Unreferenced pieces and their media are dropped on save.
  - Round trip with unknown fields.
- **Library** (`concat-host`, scratch folders as in the template tests):
  - Save → list → load → insert into a project with a different frame shape.
  - Assets land in the project with absolute paths.
  - A nested piece is flattened on save.
  - Every refusal case above.
  - Delete refuses a path outside the library.
- **In the app:** build the hand-with-whoosh example, save it, drop it into a
  vertical project, then move, rotate, scale, stretch and retitle it.
  Compare the preview with the export.

## Out of scope

- Live-linked library updates (editing the library copy updates projects).
- Choosing which properties a piece exposes.
- Pieces inside pieces.
- Editing a piece's inside without unpacking it.
- Pieces in the phone UI.
- Sharing or importing pieces between machines, beyond copying the folder.
