//! Reproduces the timing figures in the Delegated Steps spec from filed Session Timelines.
//!
//! `cargo run --example session_timing -- <evidence-root> [--compare <evidence-root>]`
//!
//! Help Request and Takeover holds are cut out of every call and gap, and a gap over two
//! minutes is left out. Nothing is sent anywhere.

#[path = "support/walk.rs"]
#[allow(
    dead_code,
    reason = "the replay uses the directory walk, this example the file walk"
)]
mod walk;

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use keek::timeline::{SessionTimeline, TimelineEvent};

const GAP_LIMIT_MS: i64 = 120_000;
const WINDOW_STATE: &str = "get_window_state";

#[derive(Default)]
struct Stats {
    sessions: usize,
    unreadable: usize,
    calls: usize,
    actions: usize,
    window_state_calls: usize,
    call_ms: Vec<i64>,
    gap_ms: Vec<i64>,
    gaps_over_limit: usize,
    overlapping_calls: usize,
}

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let Some(root) = args.next() else {
        bail!("usage: session_timing <evidence-root> [--compare <evidence-root>]");
    };
    let mut compare = None;
    while let Some(flag) = args.next() {
        match flag.as_str() {
            "--compare" => {
                compare = Some(args.next().context("--compare needs an evidence root")?);
            }
            other => bail!("unknown argument {other}. Use --compare <evidence-root>."),
        }
    }

    let first = measure(&PathBuf::from(&root))?;
    match compare {
        None => print_rows(&[(&root, &first)]),
        Some(other) => {
            let second = measure(&PathBuf::from(&other))?;
            print_rows(&[(&root, &first), (&other, &second)]);
        }
    }
    Ok(())
}

fn measure(root: &std::path::Path) -> Result<Stats> {
    let mut stats = Stats::default();
    for path in walk::files_ending(root, "_timeline.json")? {
        let text =
            fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?;
        match SessionTimeline::parse(&text) {
            Ok(timeline) => add_session(&mut stats, &timeline),
            Err(_) => stats.unreadable += 1,
        }
    }
    Ok(stats)
}

fn add_session(stats: &mut Stats, timeline: &SessionTimeline) {
    stats.sessions += 1;
    let mut calls: Vec<(i64, i64)> = Vec::new();
    let mut holds: Vec<(i64, i64)> = Vec::new();
    for event in timeline.events() {
        match event {
            TimelineEvent::Call {
                tool,
                action,
                start_ms,
                end_ms,
                ..
            } => {
                stats.calls += 1;
                stats.actions += usize::from(*action);
                stats.window_state_calls += usize::from(tool == WINDOW_STATE);
                calls.push((start_ms.0 as i64, end_ms.0 as i64));
            }
            TimelineEvent::Takeover {
                start_ms, end_ms, ..
            }
            | TimelineEvent::HelpRequest {
                start_ms, end_ms, ..
            } => holds.push((start_ms.0 as i64, end_ms.0 as i64)),
            TimelineEvent::Reminder { .. }
            | TimelineEvent::CheckMark { .. }
            | TimelineEvent::DelegatedStep(_) => {}
        }
    }
    calls.sort_unstable();
    let holds = merge(holds);

    for (start, end) in &calls {
        stats
            .call_ms
            .push(end - start - overlap(&holds, *start, *end));
    }
    for pair in calls.windows(2) {
        let (prev_end, next_start) = (pair[0].1, pair[1].0);
        if next_start < prev_end {
            stats.overlapping_calls += 1;
            continue;
        }
        let gap = next_start - prev_end - overlap(&holds, prev_end, next_start);
        if gap > GAP_LIMIT_MS {
            stats.gaps_over_limit += 1;
        } else {
            stats.gap_ms.push(gap);
        }
    }
}

fn merge(mut spans: Vec<(i64, i64)>) -> Vec<(i64, i64)> {
    spans.sort_unstable();
    let mut merged: Vec<(i64, i64)> = Vec::new();
    for (start, end) in spans {
        match merged.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => merged.push((start, end)),
        }
    }
    merged
}

fn overlap(holds: &[(i64, i64)], start: i64, end: i64) -> i64 {
    holds
        .iter()
        .map(|(hold_start, hold_end)| (end.min(*hold_end) - start.max(*hold_start)).max(0))
        .sum()
}

fn median(values: &[i64]) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    match sorted.len() {
        0 => 0.0,
        n if n % 2 == 1 => sorted[n / 2] as f64,
        n => (sorted[n / 2 - 1] + sorted[n / 2]) as f64 / 2.0,
    }
}

fn print_rows(rows: &[(&String, &Stats)]) {
    let mut lines: Vec<(&str, Vec<String>)> = vec![
        ("evidence root", vec![]),
        ("sessions read", vec![]),
        ("sessions unreadable", vec![]),
        ("calls", vec![]),
        ("actions", vec![]),
        ("gaps used", vec![]),
        ("gaps over 2 minutes, left out", vec![]),
        ("overlapping calls, no gap", vec![]),
        ("share of active time between calls", vec![]),
        ("median gap", vec![]),
        ("median call", vec![]),
        ("share of calls that are get_window_state", vec![]),
    ];
    for (root, stats) in rows {
        let gaps: i64 = stats.gap_ms.iter().sum();
        let calls: i64 = stats.call_ms.iter().sum();
        let share = |part: f64, whole: f64| {
            if whole > 0.0 {
                format!("{:.1}%", 100.0 * part / whole)
            } else {
                "n/a".to_owned()
            }
        };
        let cells = [
            (*root).clone(),
            stats.sessions.to_string(),
            stats.unreadable.to_string(),
            stats.calls.to_string(),
            stats.actions.to_string(),
            stats.gap_ms.len().to_string(),
            stats.gaps_over_limit.to_string(),
            stats.overlapping_calls.to_string(),
            share(gaps as f64, (gaps + calls) as f64),
            format!("{:.2} s", median(&stats.gap_ms) / 1000.0),
            format!("{:.2} s", median(&stats.call_ms) / 1000.0),
            share(stats.window_state_calls as f64, stats.calls as f64),
        ];
        for (line, cell) in lines.iter_mut().zip(cells) {
            line.1.push(cell);
        }
    }
    for (name, cells) in lines {
        println!("{name:<42}{}", cells.join("   "));
    }
}
