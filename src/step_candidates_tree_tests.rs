use super::{candidates_from_tree, label_at};
use crate::step_candidates::{CANDIDATE_CAP, Frame, StepAction};

const SIGN_IN_TREE: &str = r#"- [0] frame "Sample app - Browser" [actions=[doDefault,showContextMenu]]
  - [1] panel "￼￼" [actions=[doDefault,showContextMenu]]
    - [2] entry "Address and search bar" [actions=[activate,showContextMenu]]
    - [3] document web "Sign in" [actions=[doDefault,showContextMenu]]
      - [4] heading "Sign in" [actions=[clickAncestor,showContextMenu]]
        - static = "Sign in"
      - [5] link "Continue with a provider" [actions=[jump,showContextMenu]]
        - static = "Continue with "
      - [6] form "￼￼￼" [actions=[clickAncestor,showContextMenu]]
        - [7] entry "Username" [actions=[activate,showContextMenu]]
        - [8] password text "Password" [actions=[activate,showContextMenu]]
        - [9] button "Sign in" (description "Submits the form") [actions=[press,showContextMenu]]
        - [10] button "￼" [actions=[press,showContextMenu]]
        - [11] combo box "Language" value="English" [actions=[press,showContextMenu]]
"#;

fn indexes(found: &[(usize, crate::step_candidates::Candidate)]) -> Vec<usize> {
    found.iter().map(|(index, _)| *index).collect()
}

#[test]
fn tree_lines_become_candidates_keyed_by_element_index() {
    let found = candidates_from_tree(SIGN_IN_TREE, StepAction::Click, "click Sign in");

    assert_eq!(indexes(&found), [9, 2, 5, 7, 8, 11]);
    let (_, sign_in) = &found[0];
    assert_eq!(sign_in.id, "c1");
    assert_eq!(sign_in.role, "button");
    assert_eq!(sign_in.label.as_deref(), Some("Sign in"));
    assert_eq!(sign_in.element_token, "9");
    assert_eq!(
        sign_in.frame,
        Frame {
            x: 0.0,
            y: 0.0,
            w: 0.0,
            h: 0.0
        }
    );
    let ids: Vec<&str> = found.iter().map(|(_, c)| c.id.as_str()).collect();
    assert_eq!(ids, ["c1", "c2", "c3", "c4", "c5", "c6"]);
}

#[test]
fn lines_without_a_label_are_skipped_for_click() {
    let found = candidates_from_tree(SIGN_IN_TREE, StepAction::Click, "click Sign in");

    assert!(indexes(&found).contains(&9));
    assert!(!indexes(&found).contains(&10));
}

#[test]
fn elements_under_a_web_document_are_marked_as_web_content() {
    let found = candidates_from_tree(SIGN_IN_TREE, StepAction::Click, "click Sign in");

    let web: Vec<(usize, bool)> = found
        .iter()
        .map(|(index, candidate)| (*index, candidate.in_web_content))
        .collect();
    assert!(web.contains(&(9, true)));
    assert!(web.contains(&(2, false)));
}

#[test]
fn a_value_and_a_description_are_read_apart_from_the_label() {
    let found = candidates_from_tree(SIGN_IN_TREE, StepAction::Select, "select English");

    assert_eq!(indexes(&found), [11]);
    assert_eq!(found[0].1.label.as_deref(), Some("Language"));
    assert_eq!(found[0].1.value.as_deref(), Some("English"));
    let button = candidates_from_tree(SIGN_IN_TREE, StepAction::Click, "click Sign in");
    assert_eq!(button[0].1.label.as_deref(), Some("Sign in"));
}

#[test]
fn type_candidates_are_the_text_fields() {
    let found = candidates_from_tree(SIGN_IN_TREE, StepAction::Type, "type Username");

    assert_eq!(indexes(&found), [7, 2, 8]);
}

#[test]
fn at_most_the_cap_is_kept_and_the_best_match_comes_first() {
    let tree: String = (0..CANDIDATE_CAP + 6)
        .map(|index| format!("- [{index}] button \"Item {index}\" [actions=[press]]\n"))
        .collect();

    let found = candidates_from_tree(&tree, StepAction::Click, "click Item 29");

    assert_eq!(found.len(), CANDIDATE_CAP);
    assert_eq!(found[0].0, 29);
    assert_eq!(found[0].1.id, "c1");
}

#[test]
fn a_label_is_found_by_element_index_and_an_unlabelled_element_has_none() {
    assert_eq!(label_at(SIGN_IN_TREE, 9).as_deref(), Some("Sign in"));
    assert_eq!(label_at(SIGN_IN_TREE, 10), None);
    assert_eq!(label_at(SIGN_IN_TREE, 99), None);
}
