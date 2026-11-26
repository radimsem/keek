use crate::step_candidates::{Candidate, Frame, StepAction, rank_and_cap};

// Cua writes this where an element holds an embedded object instead of text.
const OBJECT_REPLACEMENT: char = '\u{fffc}';
const WEB_DOCUMENT: &str = "document web";

struct Line {
    indent: usize,
    index: usize,
    role: String,
    label: Option<String>,
    value: Option<String>,
}

/// Builds candidates from the `tree_markdown` Cua files in `before_state.json`, keyed by the
/// element index in square brackets.
///
/// The tree gives no element token and no frame, so a candidate's token is its index as text and
/// its frame is zero. It does not say which elements are disabled, so none is left out for that.
/// Ranking and the cap are the ones `candidates` applies to a live window state.
#[must_use]
pub fn candidates_from_tree(
    tree_markdown: &str,
    action: StepAction,
    intent: &str,
) -> Vec<(usize, Candidate)> {
    let mut open: Vec<(usize, bool)> = Vec::new();
    let mut found = Vec::new();
    for line in tree_markdown.lines().filter_map(parse_line) {
        while open
            .last()
            .is_some_and(|(indent, _)| *indent >= line.indent)
        {
            open.pop();
        }
        let in_web_content = open.iter().any(|(_, is_document)| *is_document);
        open.push((line.indent, line.role == WEB_DOCUMENT));

        if !action.roles().contains(&line.role.as_str())
            || (line.label.is_none() && needs_label(action))
        {
            continue;
        }
        found.push((
            line.index,
            Candidate {
                id: String::new(),
                element_token: line.index.to_string(),
                role: line.role,
                label: line.label,
                value: line.value,
                frame: Frame {
                    x: 0.0,
                    y: 0.0,
                    w: 0.0,
                    h: 0.0,
                },
                in_web_content,
            },
        ));
    }
    rank_and_cap(found, intent)
}

/// The label of the element at `index`, whatever its role, or `None` when it has none.
#[must_use]
pub fn label_at(tree_markdown: &str, index: usize) -> Option<String> {
    tree_markdown
        .lines()
        .filter_map(parse_line)
        .find(|line| line.index == index)
        .and_then(|line| line.label)
}

fn needs_label(action: StepAction) -> bool {
    match action {
        StepAction::Click | StepAction::DoubleClick | StepAction::RightClick => true,
        StepAction::Type | StepAction::Select => false,
    }
}

/// Reads `- [index] role "label" (description "...") value="..." [actions=[...]]`.
fn parse_line(line: &str) -> Option<Line> {
    let text = line.trim_start();
    let indent = line.len() - text.len();
    let (index, rest) = text.strip_prefix("- [")?.split_once("] ")?;
    let index = index.parse().ok()?;

    let mut rest = rest.rfind(" [actions=").map_or(rest, |at| &rest[..at]);
    let mut value = None;
    if rest.ends_with('"')
        && let Some(at) = rest.rfind(" value=\"")
    {
        value = clean(&rest[at + " value=\"".len()..rest.len() - 1]);
        rest = &rest[..at];
    }
    if rest.ends_with(')')
        && let Some(at) = rest.rfind(" (description \"")
    {
        rest = &rest[..at];
    }
    let (role, label) = match rest.split_once(" \"") {
        Some((role, label)) => (role, clean(label.strip_suffix('"').unwrap_or(label))),
        None => (rest, None),
    };
    Some(Line {
        indent,
        index,
        role: role.to_owned(),
        label,
        value,
    })
}

fn clean(text: &str) -> Option<String> {
    let text: String = text.chars().filter(|c| *c != OBJECT_REPLACEMENT).collect();
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

#[cfg(test)]
#[path = "step_candidates_tree_tests.rs"]
mod tests;
