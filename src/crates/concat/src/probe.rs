// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jareer and Concat contributors

//! Where a laggy playback spends its time, written to the run's log once a
//! second: the transport's tick, a full publish, a frame's round trip from
//! request to window, its drawing, its gathering on the worker, and the
//! requests folded into one while a frame was out.

use std::cell::RefCell;
use std::time::{Duration, Instant};

#[derive(Default)]
struct Stat {
    n: u32,
    sum: Duration,
    max: Duration,
}

impl Stat {
    fn add(&mut self, d: Duration) {
        self.n += 1;
        self.sum += d;
        self.max = self.max.max(d);
    }
    fn line(&self) -> String {
        if self.n == 0 {
            return "-".to_owned();
        }
        format!(
            "{}x avg {:.1} max {:.1}",
            self.n,
            self.sum.as_secs_f64() * 1e3 / f64::from(self.n),
            self.max.as_secs_f64() * 1e3
        )
    }
}

#[derive(Default)]
struct Probe {
    since: Option<Instant>,
    stats: [Stat; 6],
}

const NAMES: [&str; 6] = ["tick", "publish", "latency", "draw", "sources", "wanted"];
pub const TICK: usize = 0;
pub const PUBLISH: usize = 1;
pub const LATENCY: usize = 2;
pub const DRAW: usize = 3;
pub const SOURCES: usize = 4;
pub const WANTED: usize = 5;

thread_local! {
    static PROBE: RefCell<Probe> = RefCell::new(Probe::default());
}

pub fn add(which: usize, d: Duration) {
    PROBE.with(|probe| {
        let mut probe = probe.borrow_mut();
        let since = *probe.since.get_or_insert_with(Instant::now);
        probe.stats[which].add(d);
        if since.elapsed() >= Duration::from_secs(1) {
            let line = NAMES
                .iter()
                .zip(&probe.stats)
                .map(|(name, stat)| format!("{name}: {}", stat.line()))
                .collect::<Vec<_>>()
                .join(" | ");
            log::info!("probe {line}");
            *probe = Probe::default();
        }
    });
}
