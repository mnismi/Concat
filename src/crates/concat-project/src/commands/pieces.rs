// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Placing, retitling and unpacking pieces; see [`crate::pieces`].
//!
//! One arm per command, exactly as [`super::apply`] routes them here;
//! everything these arms share lives in the parent module.

use std::collections::HashMap;

use super::clips;
use super::*;
use crate::model::PiecePlacement;

/// Applies one of this module's commands. Any other is a routing error.
pub(super) fn apply(
    project: &mut Project,
    mint: &mut IdMint,
    command: Command,
) -> Result<Outcome, CommandError> {
    match command {
        Command::InsertPiece {
            mut piece,
            media,
            fonts,
            track_id,
            start,
        } => {
            // The bin first: by path, so the same file is one item.
            let mut ids: HashMap<String, String> = HashMap::new();
            for mut item in media {
                let before = item.id.clone();
                let id = match project.media.iter().find(|held| held.path == item.path) {
                    Some(held) => held.id.clone(),
                    None => {
                        let id = mint.next("m");
                        item.id = id.clone();
                        item.piece_media = true;
                        item.placeholder = false;
                        project.media.push(item);
                        id
                    }
                };
                ids.insert(before, id);
            }
            for font in fonts {
                if !project.fonts.iter().any(|held| held.path == font.path) {
                    project.fonts.push(font);
                }
            }
            for clip in piece
                .lanes
                .iter_mut()
                .flat_map(|lane| lane.clips.iter_mut())
            {
                if let Some(id) = ids.get(&clip.media_id) {
                    clip.media_id = id.clone();
                }
            }
            // Then the piece: the same content is the same piece.
            let piece_id = match project.pieces.iter().find(|held| {
                Piece {
                    id: held.id.clone(),
                    ..piece.clone()
                } == **held
            }) {
                Some(held) => held.id.clone(),
                None => {
                    piece.id = mint.next("p");
                    project.pieces.push(piece.clone());
                    piece.id.clone()
                }
            };
            place(project, mint, &piece_id, track_id, start)
        }

        Command::PlacePiece {
            piece_id,
            track_id,
            start,
        } => place(project, mint, &piece_id, track_id, start),

        Command::SetPieceText {
            clip_id,
            inner_clip_id,
            text,
        } => {
            let Some(clip) = project.active_mut().clip_mut(&clip_id) else {
                return Err(CommandError::ClipGone);
            };
            let Some(placement) = clip.piece.as_mut() else {
                return Ok(Outcome::default());
            };
            let applied = match text {
                Some(words) => placement.texts.insert(inner_clip_id, words.clone()) != Some(words),
                None => placement.texts.remove(&inner_clip_id).is_some(),
            };
            Ok(Outcome {
                created_id: None,
                applied,
            })
        }

        Command::UnpackPiece { clip_id } => {
            let timeline = project.active();
            let Some(piece_clip) = timeline.clip(&clip_id).cloned() else {
                return Err(CommandError::ClipGone);
            };
            let lanes = crate::pieces::expand_clip(project, &timeline.video, &piece_clip)
                .ok_or(CommandError::PieceGone)?;
            let row = timeline
                .tracks
                .iter()
                .position(|track| track.id == piece_clip.track_id)
                .unwrap_or(0);
            let timeline = project.active_mut();
            timeline.clips.retain(|clip| clip.id != clip_id);
            let mut created = None;
            for (offset, lane) in lanes.into_iter().enumerate() {
                let track_id = mint.next("t");
                timeline.tracks.insert(
                    row + offset,
                    Track {
                        id: track_id.clone(),
                        ..Track::default()
                    },
                );
                for mut clip in lane {
                    clip.id = mint.next("c");
                    clip.track_id = track_id.clone();
                    created = Some(clip.id.clone());
                    timeline.clips.push(Arc::new(clip));
                }
            }
            Ok(Outcome {
                created_id: created,
                applied: true,
            })
        }

        _ => unreachable!("commands::apply routes only this module's commands here"),
    }
}

/// One clip of the held piece `piece_id` at `start`, at its saved length.
fn place(
    project: &mut Project,
    mint: &mut IdMint,
    piece_id: &str,
    track_id: Option<String>,
    start: f64,
) -> Result<Outcome, CommandError> {
    let piece = project
        .piece(piece_id)
        .ok_or(CommandError::PieceGone)?
        .clone();
    let start = start.max(0.0);
    let timeline = project.active_mut();
    let track_id = match track_id {
        Some(id) if timeline.track(&id).is_some() => id,
        Some(_) => return Err(CommandError::TrackGone),
        None => clips::first_free_track(timeline, start, piece.duration)
            .ok_or(CommandError::NoTracks)?,
    };
    let id = mint.next("c");
    let mut clip = Clip::blank(
        id.clone(),
        track_id,
        ClipKind::Piece,
        piece.name.clone(),
        start,
        piece.duration,
    );
    clip.piece = Some(PiecePlacement {
        piece_id: piece.id,
        texts: Default::default(),
    });
    timeline.clips.push(Arc::new(clip));
    Ok(Outcome {
        created_id: Some(id),
        applied: true,
    })
}
