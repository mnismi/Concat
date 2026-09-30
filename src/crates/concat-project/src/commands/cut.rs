// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Taking spans out of a clip: what Remove Silences applies.
//!
//! Built from the hand edits, a split at each end of a span and a ripple
//! remove of what is between, so a cut this way is exactly the cut a
//! person would make with the razor and a ripple delete, keys, fades and
//! transitions included.

use super::*;

/// The clips a cut of `clip_id` lands on, `clip_id` first: the clip itself,
/// and when it is one side of a detached pair, the pieces that play with it.
/// Empty when it is not there.
///
/// A split copies a detached sound's link onto every piece, so after one
/// cut the links no longer say which piece goes with which. Pieces are
/// paired by what they show instead: the picture (a muted video of the
/// file) and one sound per stream (a clip detached from the file's
/// picture), each the one whose source overlaps the named clip's and whose
/// place on the timeline is nearest to it. A sound whose picture was
/// deleted still goes with its sibling streams.
pub fn cut_group(timeline: &Timeline, clip_id: &str) -> Vec<String> {
    let Some(named) = timeline.clip(clip_id) else {
        return Vec::new();
    };
    // `None` is the picture, `Some(stream)` a detached sound of that stream.
    let role = |clip: &Clip| -> Option<Option<Option<u32>>> {
        if clip.detached_from.is_some() {
            Some(Some(clip.audio_stream))
        } else if clip.kind == ClipKind::Video && clip.muted == Some(true) {
            Some(None)
        } else {
            None
        }
    };
    let mut group = vec![named.id.clone()];
    let Some(own) = role(named) else {
        return group;
    };
    let (from, to) = source_window(named);
    // Where source zero falls on the timeline: equal for a pair in step.
    let zero = |clip: &Clip| clip.start - clip.source_start / clip.speed;
    let mut best: Vec<(Option<Option<u32>>, f64, &Clip)> = Vec::new();
    for clip in &timeline.clips {
        let Some(key) = role(clip) else {
            continue;
        };
        if key == own || clip.media_id != named.media_id {
            continue;
        }
        let (a, b) = source_window(clip);
        if b.min(to) - a.max(from) <= MIN_CLIP_DURATION {
            continue;
        }
        let distance = (zero(clip) - zero(named)).abs();
        match best.iter_mut().find(|(held, ..)| *held == key) {
            Some(entry) if distance < entry.1 => *entry = (key, distance, clip),
            Some(_) => {}
            None => best.push((key, distance, clip)),
        }
    }
    group.extend(best.into_iter().map(|(_, _, clip)| clip.id.clone()));
    group
}

/// The source seconds `clip` shows.
fn source_window(clip: &Clip) -> (f64, f64) {
    (
        clip.source_start,
        clip.source_start + clip.duration * clip.speed,
    )
}

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
    let group: Vec<Clip> = {
        let timeline = project.active();
        if timeline.clip(&clip_id).is_none() {
            return Err(CommandError::ClipGone);
        }
        let group: Vec<Clip> = cut_group(timeline, &clip_id)
            .iter()
            .filter_map(|id| timeline.clip(id))
            .cloned()
            .collect();
        if group.iter().any(|member| member.speed_curve.is_some()) {
            return Err(CommandError::SpeedCurveCut);
        }
        group
    };
    // Only what every member shows: a span one of them lacks would take
    // time out of the others alone and slide the pair apart.
    let from = group
        .iter()
        .map(|member| source_window(member).0)
        .fold(f64::NEG_INFINITY, f64::max);
    let to = group
        .iter()
        .map(|member| source_window(member).1)
        .fold(f64::INFINITY, f64::min);
    let ranges: Vec<(f64, f64)> = ranges
        .iter()
        .map(|(a, b)| (a.min(*b).max(from), a.max(*b).min(to)))
        .filter(|(a, b)| b > a)
        .collect();

    // All or nothing, as a batch is: a refusal part way leaves no half cut.
    let mut staged = project.clone();
    let mut applied = false;
    for member in &group {
        // Where it sits now: cutting a member earlier on the same track has
        // moved it left.
        let Some(member) = staged.active().clip(&member.id).cloned() else {
            continue;
        };
        let spans = spans_on(&member, &ranges);
        if spans.is_empty() {
            continue;
        }
        let end = member.start + member.duration;
        if spans[0].0 <= member.start && spans[0].1 >= end {
            return Err(CommandError::NothingLeft);
        }
        // From the last span back, so what is still to cut never moves:
        // the named clip is always the piece left of the next cut.
        let mut kept = vec![member.id.clone()];
        for (from, to) in spans.into_iter().rev() {
            let id = member.id.clone();
            let tail = super::apply(
                &mut staged,
                mint,
                Command::SplitClips {
                    clip_ids: vec![id.clone()],
                    time: to,
                },
            )?
            .created_id;
            kept.extend(tail);
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
            kept.retain(|piece| *piece != middle);
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
        keep_the_ends(&mut staged, &member, &kept);
    }
    *project = staged;
    Ok(Outcome {
        created_id: None,
        applied,
    })
}

/// The whole's way in and fade-in on the first piece left, its fade-out on
/// the last: a leading or trailing span took the pieces that held them.
fn keep_the_ends(project: &mut Project, whole: &Clip, kept: &[String]) {
    let timeline = project.active_mut();
    let mut placed: Vec<(f64, String)> = kept
        .iter()
        .filter_map(|id| timeline.clip(id).map(|clip| (clip.start, id.clone())))
        .collect();
    placed.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (Some((_, first)), Some((_, last))) = (placed.first().cloned(), placed.last().cloned())
    else {
        return;
    };
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
