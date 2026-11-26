//! Measures how well the configured Decision Model picks the control a filed click used.
//!
//! `cargo run --example delegate_replay -- <evidence-root> [--thresholds 0.7,0.8,0.9,0.95]
//!  [--limit N] [--config <path>]`
//!
//! Each filed click by element token becomes one request: the candidates rebuilt from that
//! Action's `before_state.json`, and the intent "click {label}", which is an upper bound because
//! the label is the answer. The page text of every tree goes to the configured endpoint, so the
//! replay prints the endpoint and the count and sends nothing until you answer `y`.

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
use keek::decision_model::{Decision, DecisionModel, DecisionRequest, JevClient};
use keek::paths::Paths;
use keek::step_candidates::{Candidate, StepAction};
use keek::step_candidates_tree::{candidates_from_tree, label_at};
use keek::step_policy::{StepDecision, decide_action};
use serde_json::Value;

const USAGE: &str =
    "usage: delegate_replay <evidence-root> [--thresholds 0.7,0.8] [--limit N] [--config <path>]";
const DEFAULT_THRESHOLDS: &[f64] = &[0.7, 0.8, 0.9, 0.95];

struct Args {
    root: PathBuf,
    thresholds: Vec<f64>,
    limit: Option<usize>,
    config: Option<PathBuf>,
}

struct Sample {
    turn: String,
    intent: String,
    candidates: Vec<Candidate>,
    used_id: String,
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
        thresholds: DEFAULT_THRESHOLDS.to_vec(),
        limit: None,
        config: None,
    };
    while let Some(flag) = args.next() {
        let mut value = || args.next().with_context(|| format!("{flag} needs a value"));
        match flag.as_str() {
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
                "unknown argument {other}. The options are --thresholds, --limit and --config.\n{USAGE}"
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

    let (mut samples, skipped) = collect(&args.root)?;
    let usable = samples.len();
    if let Some(limit) = args.limit {
        samples = spread(samples, limit);
    }
    if samples.is_empty() {
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
    println!(
        "Sending {} trees, with their page text, to {} (model {}).",
        samples.len(),
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

    let mut results: Vec<Result<Decision, String>> = Vec::new();
    for sample in &samples {
        let request = DecisionRequest {
            intent: sample.intent.clone(),
            action: StepAction::Click,
            window_title: None,
            candidates: sample.candidates.clone(),
        };
        let result = client.decide(request).await.map_err(|e| e.to_string());
        if let Err(message) = &result {
            eprintln!("{}: {message}", sample.turn);
        }
        results.push(result);
    }
    report(&samples, &results, &args.thresholds);
    Ok(())
}

fn collect(root: &Path) -> Result<(Vec<Sample>, Skipped)> {
    let mut samples = Vec::new();
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
            let intent = format!("click {label}");
            let candidates = candidates_from_tree(&tree, StepAction::Click, &intent);
            let Some((_, used)) = candidates.iter().find(|(at, _)| *at == index) else {
                skipped.not_offered += 1;
                continue;
            };
            samples.push(Sample {
                turn: turn.display().to_string(),
                intent,
                used_id: used.id.clone(),
                candidates: candidates.into_iter().map(|(_, c)| c).collect(),
            });
        }
    }
    Ok((samples, skipped))
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

/// Keeps `limit` samples spread evenly over the whole list, so a small run is not all one project.
fn spread(samples: Vec<Sample>, limit: usize) -> Vec<Sample> {
    let total = samples.len();
    if limit >= total {
        return samples;
    }
    let keep: Vec<usize> = (0..limit).map(|i| i * total / limit).collect();
    samples
        .into_iter()
        .enumerate()
        .filter(|(at, _)| keep.contains(at))
        .map(|(_, sample)| sample)
        .collect()
}

fn report(samples: &[Sample], results: &[Result<Decision, String>], thresholds: &[f64]) {
    let mut latencies: Vec<Duration> = results
        .iter()
        .filter_map(|result| result.as_ref().ok().map(|decision| decision.latency))
        .collect();
    latencies.sort_unstable();
    let failed = results.iter().filter(|result| result.is_err()).count();
    println!(
        "\n{} requests, {failed} failed. A failed request counts as handed back. Latency covers the answered ones, so it is the same on every row.",
        samples.len()
    );
    let answered = samples
        .iter()
        .zip(results)
        .filter_map(|(sample, result)| Some((sample, result.as_ref().ok()?)))
        .collect::<Vec<_>>();
    let right = answered
        .iter()
        .filter(|(sample, decision)| decision.choice == sample.used_id)
        .count();
    if answered.is_empty() {
        println!("Top-1 accuracy over answered requests: n/a (none answered).");
    } else {
        println!(
            "Top-1 accuracy over answered requests, before any threshold: {right}/{} = {:.1}%.",
            answered.len(),
            100.0 * right as f64 / answered.len() as f64
        );
    }
    println!("threshold  acted  correct  correct/all      precision  handed back  p50 ms  p95 ms");
    for &threshold in thresholds {
        let (mut acted, mut correct) = (0usize, 0usize);
        for (sample, result) in samples.iter().zip(results) {
            let Ok(decision) = result else { continue };
            if let StepDecision::Act { candidate, .. } =
                decide_action(decision, &sample.candidates, threshold)
            {
                acted += 1;
                correct += usize::from(candidate.id == sample.used_id);
            }
        }
        let total = samples.len() as f64;
        let precision = if acted == 0 {
            "n/a".to_owned()
        } else {
            format!("{:.1}%", 100.0 * correct as f64 / acted as f64)
        };
        println!(
            "{threshold:<9.2}  {acted:<5}  {correct:<7}  {:<14}  {precision:<9}  {:<11}  {:<6}  {:<6}",
            format!("{:.1}%", 100.0 * correct as f64 / total),
            format!("{:.1}%", 100.0 * (samples.len() - acted) as f64 / total),
            percentile(&latencies, 50),
            percentile(&latencies, 95),
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
