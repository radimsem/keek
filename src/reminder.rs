use crate::checklist::Checklist;
use serde_json::{Value, json};

pub(crate) const THRESHOLD: usize = 8;

#[derive(Default)]
pub(crate) struct Reminder {
    calls: usize,
}

impl Reminder {
    pub(crate) fn reset(&mut self) {
        self.calls = 0;
    }

    pub(crate) fn record_call(&mut self, checklist: &Checklist, result: &mut Value) -> bool {
        let reminded = self.calls >= THRESHOLD && append(checklist, result);
        if reminded {
            self.reset();
        }
        self.calls = self.calls.saturating_add(1);
        reminded
    }
}

fn append(checklist: &Checklist, result: &mut Value) -> bool {
    let Some(result) = result.as_object_mut() else {
        return false;
    };
    let checks = checklist
        .unmarked()
        .map(|check| {
            let text = check.text.split_whitespace().collect::<Vec<_>>().join(" ");
            format!("{}: {text}", check.id.0)
        })
        .collect::<Vec<_>>();
    if checks.is_empty() {
        return false;
    }
    let Some(content) = result
        .entry("content")
        .or_insert_with(|| Value::Array(Vec::new()))
        .as_array_mut()
    else {
        return false;
    };
    let text = format!(
        "keek: Checks not marked yet: {}. Call mark_check for each one you have already observed.",
        checks.join(", ")
    );
    content.push(json!({"type":"text","text":text}));
    true
}
