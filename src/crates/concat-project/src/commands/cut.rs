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
