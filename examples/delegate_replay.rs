//! Measures how well the configured Decision Model picks the control a filed click used.
//!
//! `cargo run --example delegate_replay -- <evidence-root> [--modes label,partial,absent]
//!  [--thresholds 0.5,0.7,0.8,0.9,0.95,0.99] [--limit N] [--config <path>]`
//!
//! Each filed click by element token gives one request per mode, with the candidates rebuilt
//! from that Action's `before_state.json`. The modes write the intent differently:
//!
//! - `label`: "click {label}". An upper bound, because the label is the answer.
//! - `partial`: "click the {word} {role}", with only the label's longest word, so controls that
//!   share the word compete.
//! - `absent`: "click {label}" with a label from another tree that this one holds nowhere. No
//!   offered control is meant, so the right outcome is handing the step back.
//!
//! The page text of every tree goes to the configured endpoint, so the replay prints the endpoint
//! and the count and sends nothing until you answer `y`.

#[path = "support/walk.rs"]
#[allow(
    dead_code,
    reason = "the timing example uses the file walk, this one the directory walk"
)]
mod walk;

use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use keek::config::Config;
use keek::decision_model::{
    ABSTAIN, Decision, DecisionModel, DecisionRequest, JevClient, REOBSERVE,
};
use keek::paths::Paths;
use keek::step_candidates::{Candidate, StepAction};
use keek::step_candidates_tree::{candidates_from_tree, label_at};
use keek::step_replay::{Expected, Outcome, absent_label, outcome, partial_intent};
use serde_json::Value;

const USAGE: &str = "usage: delegate_replay <evidence-root> [--modes label,partial,absent] [--thresholds 0.7,0.8] [--limit N] [--config <path>]";
const DEFAULT_THRESHOLDS: &[f64] = &[0.5, 0.7, 0.8, 0.9, 0.95, 0.99];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Label,
    Partial,
    Absent,
}

impl Mode {
    const ALL: [Self; 3] = [Self::Label, Self::Partial, Self::Absent];

    fn as_str(self) -> &'static str {
        match self {
            Self::Label => "label",
            Self::Partial => "partial",
            Self::Absent => "absent",
        }
    }
}

struct Args {
    root: PathBuf,
    modes: Vec<Mode>,
    thresholds: Vec<f64>,
    limit: Option<usize>,
    config: Option<PathBuf>,
}

/// A filed click by element token whose element is a click candidate in its own tree.
struct Click {
    turn: String,
    tree: String,
    index: usize,
    role: String,
    label: String,
}

struct Run {
    mode: Mode,
    samples: Vec<Sample>,
    results: Vec<Result<Decision, String>>,
}

struct Sample {
    turn: String,
    intent: String,
    candidates: Vec<Candidate>,
    expected: Expected,
}

#[derive(Default)]
struct Skipped {
    no_state: usize,
    no_element: usize,
    no_label: usize,
    not_offered: usize,
}

fn parse_args() -> Result<Args> {
    let mut args = std::env::args().skip(1);
    let root = args.next().context(USAGE)?;
    let mut parsed = Args {
        root: PathBuf::from(root),
        modes: Mode::ALL.to_vec(),
        thresholds: DEFAULT_THRESHOLDS.to_vec(),
        limit: None,
        config: None,
    };
    while let Some(flag) = args.next() {
        let mut value = || args.next().with_context(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--modes" => {
                parsed.modes = value()?
                    .split(',')
                    .map(|part| {
                        Mode::ALL
                            .into_iter()
                            .find(|mode| mode.as_str() == part.trim())
                            .with_context(|| {
                                format!(
                                    "unknown mode {part}. The modes are label, partial and absent."
                                )
                            })
                    })
                    .collect::<Result<_>>()?;
            }
            "--thresholds" => {
                parsed.thresholds = value()?
                    .split(',')
                    .map(|part| part.trim().parse::<f64>())
                    .collect::<Result<_, _>>()
                    .context("--thresholds takes numbers between 0 and 1, such as 0.8,0.9")?;
                if parsed.thresholds.iter().any(|t| !(0.0..=1.0).contains(t)) {
                    bail!("--thresholds takes numbers between 0 and 1, such as 0.8,0.9");
                }
            }
            "--limit" => {
                let limit: usize = value()?
                    .parse()
                    .context("--limit takes a whole number of at least 1")?;
                if limit == 0 {
                    bail!(
                        "--limit takes a whole number of at least 1, since 0 would send nothing."
                    );
                }
                parsed.limit = Some(limit);
            }
            "--config" => parsed.config = Some(PathBuf::from(value()?)),
            other => bail!(
                "unknown argument {other}. The options are --modes, --thresholds, --limit and --config.\n{USAGE}"
            ),
        }
    }
    Ok(parsed)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args()?;
    let config_path = match &args.config {
        Some(path) => path.clone(),
        None => Paths::from_env()?.config_file().to_path_buf(),
    };
    let config = Config::read(&config_path).await?;
    let Some(model) = config.decision_model else {
        bail!(
            "{} has no [decision_model] table. Add endpoint and model there, or pass --config <path>.",
            config_path.display()
        );
    };
    let client = JevClient::new(&model)?;

    let (mut clicks, skipped) = collect(&args.root)?;
    let usable = clicks.len();
    if let Some(limit) = args.limit {
        clicks = spread(clicks, limit);
    }
    if clicks.is_empty() {
        bail!(
            "no filed click by element token has a usable tree under {}",
            args.root.display()
        );
    }
    println!(
        "{usable} usable trees, and {} clicks skipped (no before state {}, element not in the tree {}, no label {}, not a click candidate {}).",
        skipped.no_state + skipped.no_element + skipped.no_label + skipped.not_offered,
        skipped.no_state,
        skipped.no_element,
        skipped.no_label,
        skipped.not_offered,
    );
    let runs: Vec<(Mode, Vec<Sample>)> = args
        .modes
        .iter()
        .map(|&mode| (mode, samples(mode, &clicks)))
        .collect();
    for (mode, samples) in &runs {
        println!(
            "Mode {}: {} of {} trees give a request.",
            mode.as_str(),
            samples.len(),
            clicks.len()
        );
    }
    let total: usize = runs.iter().map(|(_, samples)| samples.len()).sum();
    println!(
        "Sending {total} requests, with the page text of {} trees, to {} (model {}).",
        clicks.len(),
        model.endpoint,
        model.model
    );
    print!("Send them? Type y to go on, anything else stops: ");
    io::stdout().flush()?;
    let mut answer = String::new();
    io::stdin().lock().read_line(&mut answer)?;
    if answer.trim() != "y" {
        println!("Nothing was sent.");
        return Ok(());
    }

    let mut answered: Vec<Run> = Vec::new();
    for (mode, samples) in runs {
        let mut results = Vec::new();
        for sample in &samples {
            let request = DecisionRequest {
                intent: sample.intent.clone(),
                action: StepAction::Click,
                window_title: None,
                candidates: sample.candidates.clone(),
            };
            let result = client.decide(request).await.map_err(|e| e.to_string());
            if let Err(message) = &result {
                eprintln!("{} ({}): {message}", sample.turn, mode.as_str());
            }
            results.push(result);
        }
        answered.push(Run {
            mode,
            samples,
            results,
        });
    }
    for run in &answered {
        report(run, &args.thresholds);
    }
    if answered.len() > 1 {
        report_all(&answered, &args.thresholds);
    }
    Ok(())
}

fn collect(root: &Path) -> Result<(Vec<Click>, Skipped)> {
    let mut clicks = Vec::new();
    let mut skipped = Skipped::default();
    for actions in walk::dirs_ending(root, "_actions")? {
        let mut turns: Vec<PathBuf> = fs::read_dir(&actions)?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| path.is_dir())
            .collect();
        turns.sort();
        for turn in turns {
            let Some(index) = clicked_index(&turn) else {
                continue;
            };
            let Some(tree) = tree_markdown(&turn) else {
                skipped.no_state += 1;
                continue;
            };
            let Some(label) = label_at(&tree, index) else {
                if tree.contains(&format!("- [{index}] ")) {
                    skipped.no_label += 1;
                } else {
                    skipped.no_element += 1;
                }
                continue;
            };
            let candidates =
                candidates_from_tree(&tree, StepAction::Click, &format!("click {label}"));
            let Some((_, used)) = candidates.iter().find(|(at, _)| *at == index) else {
                skipped.not_offered += 1;
                continue;
            };
            clicks.push(Click {
                turn: turn.display().to_string(),
                role: used.role.clone(),
                tree,
                index,
                label,
            });
        }
    }
    Ok((clicks, skipped))
}

/// One request per click that `mode` can write an intent for.
fn samples(mode: Mode, clicks: &[Click]) -> Vec<Sample> {
    clicks
        .iter()
        .enumerate()
        .filter_map(|(at, click)| {
            let intent = match mode {
                Mode::Label => format!("click {}", click.label),
                Mode::Partial => partial_intent(&click.role, &click.label)?,
                Mode::Absent => {
                    // Starting half way round takes the other label from a distant tree, which
                    // is usually another Project.
                    let others = (0..clicks.len())
                        .map(|step| (at + clicks.len() / 2 + step) % clicks.len())
                        .filter(|&other| other != at)
                        .filter_map(|other| clicks.get(other))
                        .map(|other| other.label.as_str());
                    format!("click {}", absent_label(&click.tree, others)?)
                }
            };
            let candidates = candidates_from_tree(&click.tree, StepAction::Click, &intent);
            let expected = match mode {
                Mode::Absent => Expected::HandBack,
                // A control the ranking dropped can't be picked, so handing back is right.
                Mode::Label | Mode::Partial => candidates
                    .iter()
                    .find(|(index, _)| *index == click.index)
                    .map_or(Expected::HandBack, |(_, used)| {
                        Expected::Control(used.id.clone())
                    }),
            };
            Some(Sample {
                turn: click.turn.clone(),
                intent,
                candidates: candidates.into_iter().map(|(_, c)| c).collect(),
                expected,
            })
        })
        .collect()
}

/// The element index a filed `click` by element token used, or `None` for any other Action.
fn clicked_index(turn: &Path) -> Option<usize> {
    let action: Value =
        serde_json::from_str(&fs::read_to_string(turn.join("action.json")).ok()?).ok()?;
    if action.get("tool")?.as_str()? != "click" {
        return None;
    }
    let token = action.get("arguments")?.get("element_token")?.as_str()?;
    token.rsplit_once(':')?.1.parse().ok()
}

fn tree_markdown(turn: &Path) -> Option<String> {
    let state: Value =
        serde_json::from_str(&fs::read_to_string(turn.join("before_state.json")).ok()?).ok()?;
    Some(state.get("tree_markdown")?.as_str()?.to_owned())
}

/// Keeps `limit` clicks spread evenly over the whole list, so a small run is not all one project.
fn spread(clicks: Vec<Click>, limit: usize) -> Vec<Click> {
    let total = clicks.len();
    if limit >= total {
        return clicks;
    }
    let keep: Vec<usize> = (0..limit).map(|i| i * total / limit).collect();
    clicks
        .into_iter()
        .enumerate()
        .filter(|(at, _)| keep.contains(at))
        .map(|(_, click)| click)
        .collect()
}

fn report(run: &Run, thresholds: &[f64]) {
    let Run {
        mode,
        samples,
        results,
    } = run;
    if samples.is_empty() {
        println!(
            "\nMode {}: no click gave this mode an intent, so nothing was sent.",
            mode.as_str()
        );
        return;
    }
    let failed = results.iter().filter(|result| result.is_err()).count();
    println!(
        "\nMode {}: {} requests, {failed} failed. A failed request counts as handed back.",
        mode.as_str(),
        samples.len()
    );
    let answered: Vec<(&Sample, &Decision)> = samples
        .iter()
        .zip(results)
        .filter_map(|(sample, result)| Some((sample, result.as_ref().ok()?)))
        .collect();
    let first = samples
        .iter()
        .filter(|sample| sample.expected == Expected::Control("c1".to_owned()))
        .count();
    let dropped = samples
        .iter()
        .filter(|sample| sample.expected == Expected::HandBack)
        .count();
    let declined = answered
        .iter()
        .filter(|(_, decision)| [ABSTAIN, REOBSERVE].contains(&decision.choice.as_str()))
        .count();
    match mode {
        Mode::Label | Mode::Partial => {
            let right = answered
                .iter()
                .filter(|(sample, decision)| {
                    sample.expected == Expected::Control(decision.choice.clone())
                })
                .count();
            println!(
                "Before any threshold: top-1 {right}/{} answered, abstain or reobserve {declined}. Ranking put the used control first in {first}/{} and dropped it from the candidates in {dropped}.",
                answered.len(),
                samples.len()
            );
        }
        Mode::Absent => println!(
            "Before any threshold: abstain or reobserve {declined}/{} answered. The rest picked a control, and none is meant.",
            answered.len()
        ),
    }
    table(&answered, samples.len(), thresholds);
    wrong_acts(&answered, thresholds);
}

fn report_all(answered: &[Run], thresholds: &[f64]) {
    let all: Vec<(&Sample, &Decision)> = answered
        .iter()
        .flat_map(|run| {
            run.samples
                .iter()
                .zip(&run.results)
                .filter_map(|(sample, result)| Some((sample, result.as_ref().ok()?)))
        })
        .collect();
    let total = answered.iter().map(|run| run.samples.len()).sum();
    println!("\nAll modes together: {total} requests.");
    table(&all, total, thresholds);
}

/// Right and wrong count acts. Every request that did not act, failed ones included, is handed back.
fn table(answered: &[(&Sample, &Decision)], total: usize, thresholds: &[f64]) {
    let mut latencies: Vec<Duration> = answered.iter().map(|(_, d)| d.latency).collect();
    latencies.sort_unstable();
    println!("threshold  acted  right  wrong  right/all  handed back  p50 ms  p95 ms");
    for &threshold in thresholds {
        let (mut right, mut wrong) = (0usize, 0usize);
        for (sample, decision) in answered {
            match outcome(decision, &sample.candidates, &sample.expected, threshold) {
                Outcome::RightAct => right += 1,
                Outcome::WrongAct => wrong += 1,
                Outcome::HandedBack => {}
            }
        }
        let acted = right + wrong;
        let share = |count: usize| format!("{:.1}%", 100.0 * count as f64 / total as f64);
        println!(
            "{threshold:<9.2}  {acted:<5}  {right:<5}  {wrong:<5}  {:<9}  {:<11}  {:<6}  {}",
            share(right),
            share(total - acted),
            percentile(&latencies, 50),
            percentile(&latencies, 95),
        );
    }
}

/// Lists every wrong act at the lowest threshold, with the numbers that decide which thresholds
/// would have stopped it, so a fair pick (a synonym, say) can be told from a real mistake.
fn wrong_acts(answered: &[(&Sample, &Decision)], thresholds: &[f64]) {
    let Some(lowest) = thresholds.iter().copied().reduce(f64::min) else {
        return;
    };
    let wrong: Vec<_> = answered
        .iter()
        .filter(|(sample, decision)| {
            outcome(decision, &sample.candidates, &sample.expected, lowest) == Outcome::WrongAct
        })
        .collect();
    if wrong.is_empty() {
        return;
    }
    println!("Wrong acts at {lowest:.2}:");
    for (sample, decision) in wrong {
        let picked = sample
            .candidates
            .iter()
            .find(|candidate| candidate.id == decision.choice)
            .map_or_else(String::new, |c| {
                format!("{} \"{}\"", c.role, c.label.as_deref().unwrap_or(""))
            });
        println!(
            "  {} -> {picked}, pick {:.3}, matches {:.3} ({})",
            sample.intent,
            decision
                .probabilities
                .get(&decision.choice)
                .copied()
                .unwrap_or(0.0),
            decision.matches,
            sample.turn
        );
    }
}

fn percentile(sorted: &[Duration], percent: usize) -> String {
    if sorted.is_empty() {
        return "n/a".to_owned();
    }
    let rank = (percent * sorted.len())
        .div_ceil(100)
        .clamp(1, sorted.len());
    format!("{}", sorted[rank - 1].as_millis())
}
