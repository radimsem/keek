# Delegated Steps Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The Verifier can hand keek a routine step as a `delegate_step` call. keek asks a Jev-compatible Decision Model which control the intent means, and acts through Cua only when the model is confident.

**Architecture:** The session actor intercepts `delegate_step` before forwarding. It:
1. calls Cua's `get_window_state` once;
2. builds candidates from that capture's elements;
3. sends one masked request with a Choice question and a Noul question through an injected `DecisionModel`;
4. passes the answer through a pure policy;
5. either calls Cua's `click`, `type_text` or `set_value` with the candidate's element token, or hands the top candidates back.

Every step adds a `DelegatedStep` event to the Session Timeline.

**Tech Stack:** Rust (tokio, serde_json, thiserror, rmcp 3.8). New dependency: `reqwest` with `default-features = false` and features `json` and `rustls-tls`. It is the HTTP client for the Jev API, and the PR says why.

**Spec:** `docs/specs/2026-11-26-delegated-steps.md`. Jev API: https://docs.typesafe.ai/api.md, https://docs.typesafe.ai/primitives/choice.md and https://docs.typesafe.ai/primitives/noul.md. Read all three before Task 3.

## Global Constraints

- Gates on every commit: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`. Clippy denies `unwrap`, `expect`, `panic` and `todo`.
- One `thiserror` enum per module, and every message ends with what to do. `anyhow` stays in `main.rs`, `mcp.rs` and tests.
- Match keek's own enums exhaustively, with no `_` arm.
- Off until `[decision_model]` is configured. Without it, `delegate_step` is not listed and every existing test passes unchanged.
- A Test Credential's secret value never reaches the Decision Model. `${NAME}` is substituted only in the Cua action arguments, after the decision. The state sent is masked with `Credentials::mask` and `mask_value`.
- The API key is read only from the environment variable named by `api_key_env`. It never enters config output, tool results, the Session Timeline, logs, test fixtures or commits. Tests use made-up keys.
- keek acts only when both of these hold:
  - the pick is one of the offered candidate ids, and its probability is at least `threshold`;
  - the Noul probability is at least `threshold`.
  Otherwise it hands back the top candidates and does nothing. keek never retries a step on its own.
- A Decision Model never marks a Check. `mark_check` stays the Verifier's (ADR 0012).
- An unreachable endpoint, a timeout or a malformed answer hands back the candidates. The session goes on.
- Source files stay under about 20 KB. Commits are subject only, conventional, at most 72 characters. Branch: `feat/delegated-steps`.
- Nothing goes to `trycua/cua`. No replay run against the hosted endpoint until the developer says yes to sending past trees.

## Review Focus

1. **Nothing on screen matches.** The model answers `abstain`, or every candidate has a low probability. The result says no action was taken and lists the top candidates. Test in Task 5.
2. **The intent names a Test Credential.** For example "type `${TEST_PASSWORD}` into the password field". The secret is never in the request body sent to the model, and the timeline event holds only `${TEST_PASSWORD}`. Test in Task 5.
3. **A window with more controls than the cap.** A page with 300 links gets the best 24 by intent overlap. The right one is still offered when its label matches the intent. Test in Task 2.
4. **The model is slow or down.** A timeout returns within `timeout_ms` plus a small margin, as a handed-back step, and the next tool call works. Test in Task 5 with a fake that never answers.
5. **The model picks an id that was never offered.** For example, an out-of-range index. keek doesn't act. Test in Task 4.

---

### Task 1: The `[decision_model]` config

**Files:**
- Modify: `src/config.rs` (and its tests)

**Interfaces:**
- Produces:
  - `pub decision_model: Option<DecisionModelConfig>` on `Config`.
  - `#[derive(Debug, Clone, Deserialize)] #[serde(deny_unknown_fields)] pub struct DecisionModelConfig`, with fields:
    - `pub endpoint: String`
    - `pub model: String`
    - `#[serde(default)] pub api_key_env: Option<String>`
    - `#[serde(default = "default_threshold")] pub threshold: f64`, default `0.9`
    - `#[serde(default = "default_timeout_ms")] pub timeout_ms: u64`, default `1500`
  - New `ConfigError` variants: `Threshold { path, value }` and `Endpoint { path, endpoint }`.

- [ ] **Step 1: Write the failing tests**
  - `a_config_without_decision_model_leaves_it_off`: `Config::parse("session_cap = 2")` gives `decision_model.is_none()`.
  - `a_decision_model_table_parses_with_defaults`: endpoint and model only give threshold `0.9`, timeout `1500` and no key env.
  - `a_threshold_outside_zero_to_one_is_refused`: `threshold = 1.5` gives `ConfigError::Threshold`, whose message names the file.
  - `an_endpoint_that_is_not_http_is_refused`: `endpoint = "api.typesafe.ai"` gives `ConfigError::Endpoint`.
  - `an_unknown_key_in_decision_model_is_refused`: `api_key = "x"` fails to parse, so a key can't be put in config by mistake.

- [ ] **Step 2: Run them to see them fail**
  Run: `cargo test --lib config`
  Expected: compile errors.

- [ ] **Step 3: Implement.** Validate in `Config::read` next to the `ZeroCap` check. The `Endpoint` check is only that the value starts with `http://` or `https://`. Each message ends "Fix the file and restart keek."

- [ ] **Step 4: Run the tests to see them pass**
  Run: `cargo test --lib config`
  Expected: pass.

- [ ] **Step 5: Commit**
  `git commit -m "feat(config): read an optional decision_model table"`

### Task 2: Candidates from a window state

**Files:**
- Create: `src/step_candidates.rs`, `src/step_candidates_tests.rs`, `tests/fixtures/window_state_signin.json`
- Modify: `src/lib.rs`

**Interfaces:**
- Produces:
  - `#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)] #[serde(rename_all = "snake_case")] pub enum StepAction { Click, DoubleClick, RightClick, Type, Select }`
  - `#[derive(Debug, Clone, Serialize)] pub struct Candidate`, with fields:
    - `pub id: String`, which is `"c1"` to `"c24"`
    - `pub element_token: String`, never sent to the model
    - `pub role: String`
    - `pub label: Option<String>`
    - `pub value: Option<String>`
    - `pub frame: Frame`
    - `pub in_web_content: bool`
  - `pub const CANDIDATE_CAP: usize = 24;`
  - `pub fn candidates(state: &Value, action: StepAction, intent: &str) -> Result<Vec<Candidate>, CandidatesError>`
  - `CandidatesError::NoElements` says: "get_window_state returned no element list for this window. Call get_window_state and act on the window yourself."

- [ ] **Step 1: Write the fixture.** It is a `get_window_state` result shaped like Cua 0.38.0's `WindowStateOutput`, in `structuredContent`, with a `snapshot_id` and these elements:
  - a "Sign in" button with a token
  - a disabled "Sign in with SSO" button
  - an email entry with `in_web_content: true`
  - a "Forgot password?" link
  - a text element without a token
  - a combo box with options

  Copy field names exactly from `cua-driver-contract/src/windows.rs` at tag `cua-driver-rs-v0.38.0`.

- [ ] **Step 2: Write the failing tests**
  - `click_candidates_are_enabled_elements_with_a_token_and_a_frame`: the disabled button and the token-less text are absent.
  - `web_content_elements_are_offered`: the email entry is present with `in_web_content == true`.
  - `type_candidates_are_text_entries_only`: `StepAction::Type` gives only entry roles.
  - `select_candidates_are_combo_boxes_and_lists`.
  - `candidates_matching_the_intent_come_first`: the intent "click Sign in" gives the "Sign in" button as `c1`.
  - `more_than_the_cap_keeps_the_best_matches`: build 300 links in code, with "Pricing" at index 280. The intent "click Pricing" gives 24 candidates, including "Pricing".
  - `a_state_without_elements_is_an_error`.

- [ ] **Step 3: Run them to see them fail**
  Run: `cargo test --lib step_candidates`
  Expected: compile errors.

- [ ] **Step 4: Implement**
  - Role sets per action come from the roles in the fixture. Check the AT-SPI role strings Cua emits in `~/Videos/keek/*/*/*_actions/*/before_state.json` with `jq`.
  - Ranking: the count of lowercase intent words found in the label or value, highest first, then document order.
  - Ids are assigned after the cap.

- [ ] **Step 5: Run the tests to see them pass**
  Run: `cargo test --lib step_candidates`
  Expected: pass.

- [ ] **Step 6: Commit**
  `git commit -m "feat(delegate): build step candidates from a window state"`

### Task 3: The Decision Model client

**Files:**
- Create: `src/decision_model.rs`, `src/decision_model_tests.rs`, `tests/fixtures/jev_answer.json`, `tests/live_jev.rs`
- Modify: `Cargo.toml`, `Cargo.lock`, `src/lib.rs`

**Interfaces:**
- Consumes: `Candidate`, `StepAction`, `DecisionModelConfig`.
- Produces:
  - `pub trait DecisionModel: Send + Sync { fn decide(&self, request: DecisionRequest) -> DecisionFuture; }`, where `DecisionFuture = Pin<Box<dyn Future<Output = Result<Decision, DecisionModelError>> + Send>>`. This follows `DriverFuture` in `driver.rs`.
  - `pub struct DecisionRequest`, with fields:
    - `pub intent: String`, already masked
    - `pub action: StepAction`
    - `pub window_title: Option<String>`
    - `pub candidates: Vec<Candidate>`
  - `pub struct Decision`, with fields:
    - `pub choice: String`, a candidate id, `"abstain"` or `"reobserve"`
    - `pub probabilities: BTreeMap<String, f64>`
    - `pub matches: f64`, the Noul probability
    - `pub latency: Duration`
  - `pub fn build_request(model: &str, request: &DecisionRequest) -> Value`
  - `pub fn parse_answer(body: &Value) -> Result<(String, BTreeMap<String, f64>, f64), DecisionModelError>`
  - `pub struct JevClient`, with `pub fn new(config: &DecisionModelConfig) -> Result<Self, DecisionModelError>`. It reads the key from `std::env::var(api_key_env)` once and keeps it out of `Debug`.
  - `DecisionModelError` has these variants: `MissingKey { var }`, `Timeout { ms }`, `Http { status }`, `Unreachable { endpoint }` and `Malformed { reason }`. Messages never carry the key or the request body.

- [ ] **Step 1: Read the Jev docs** (api.md, choice.md and noul.md). Write the request and answer field names into the golden fixture `tests/fixtures/jev_answer.json`, as a real answer shape.

- [ ] **Step 2: Write the failing tests**
  - `the_request_has_one_choice_and_one_noul_question_over_one_state`
  - `choice_options_are_the_candidate_ids_plus_reobserve_and_abstain`
  - `element_tokens_are_not_in_the_request`
  - `parse_answer_reads_the_choice_its_probabilities_and_the_noul`: from the fixture.
  - `parse_answer_refuses_a_body_without_the_choice`: gives `Malformed`.
  - `a_missing_key_variable_is_an_error_naming_the_variable_not_its_value`
  - `the_client_debug_output_hides_the_key`

- [ ] **Step 3: Run them to see them fail**
  Run: `cargo test --lib decision_model`
  Expected: compile errors.

- [ ] **Step 4: Implement.**
  - Add `reqwest = { version = "<current>", default-features = false, features = ["json", "rustls-tls"] }`. `Cargo.lock` changes in this commit only.
  - The question wording follows the typesafe skill's guidance:
    - **Choice:** "Which control does the Verifier's intent refer to?". Its criteria describe each candidate by role, label and value. `reobserve` means "the screen may still be changing". `abstain` means "none of these is meant".
    - **Noul:** "One of the candidates is the control the intent refers to."
  - The client timeout is `timeout_ms`.
  - `latency` is measured around the HTTP call.
  - With no `api_key_env`, no Authorization header is sent, which is the local-server case.

- [ ] **Step 5: Write `live_jev_picks_sign_in_from_the_fixture`** (`#[ignore]`). It skips with a message when `KEEK_JEV_KEY` is unset, and sends only the made-up fixture state. Run it only with the developer's go-ahead, which also needs the key exported in the shell:
  `cargo test --test live_jev -- --ignored`
  Expected: choice `c1`, with probability at least 0.9.

- [ ] **Step 6: Run the tests to see them pass**
  Run: `cargo test --lib decision_model`
  Expected: pass.

- [ ] **Step 7: Commit**
  `git commit -m "feat(delegate): ask a Jev-compatible decision model for a step"`

### Task 4: The act-or-hand-back policy

**Files:**
- Create: `src/step_policy.rs` (inline `mod tests`)

**Interfaces:**
- Consumes: `Decision`, `Candidate`.
- Produces:
  - `pub enum StepDecision { Act { candidate: Candidate, probability: f64 }, HandBack(HandBackReason) }`
  - `pub enum HandBackReason { Abstained, Reobserve, LowConfidence { probability: f64 }, NoMatch { matches: f64 }, NotOffered { choice: String }, ModelFailed(String) }`
  - `pub fn decide_action(decision: &Decision, candidates: &[Candidate], threshold: f64) -> StepDecision`

- [ ] **Step 1: Write the failing tests**
  - `a_confident_offered_pick_with_a_confident_match_acts`
  - `a_pick_below_the_threshold_hands_back`
  - `a_low_noul_hands_back_even_with_a_confident_pick`
  - `abstain_hands_back_as_abstained`
  - `reobserve_hands_back_as_reobserve`
  - `a_pick_that_was_not_offered_hands_back_and_never_acts`: choice `"c99"` with 0.99.

- [ ] **Step 2: Run them to see them fail**
  Run: `cargo test --lib step_policy`
  Expected: fail.

- [ ] **Step 3: Implement** `decide_action` as written in the Interfaces block.

- [ ] **Step 4: Run the tests to see them pass**
  Run: `cargo test --lib step_policy`
  Expected: pass.

- [ ] **Step 5: Commit**
  `git commit -m "feat(delegate): act only on a confident offered pick"`

### Task 5: `delegate_step` in the session actor, with Evidence

**Files:**
- Create: `src/session_delegate.rs`, `src/session_actor_tests/delegate.rs`, `src/session_actor_tests/fake_decision_model.rs`
- Modify:
  - `src/timeline.rs:40-73` (new variant), plus every exhaustive match on `TimelineEvent`:
    - `timeline.rs:128`
    - `session_journal.rs:113`
    - `editor/plan.rs:57` and `:134`
    - `session_end.rs:107`
    - `title_card.rs:57`
  - `src/credential_scrub.rs`
  - `src/report.rs`, with counts in `render`
  - the actor's request routing, next to `forward_call` in `src/session_call.rs`
  - `src/session.rs`, so the actor holds an `Option<Arc<dyn DecisionModel>>` and its `DecisionModelConfig`

**Interfaces:**
- Consumes: Tasks 2 to 4, `call_driver(driver, tool, args, limit)` (`session.rs:231`), `Credentials::substitute`, `mask` and `mask_value`, and the gate that `forward_call` uses.
- Produces:
  - `TimelineEvent::DelegatedStep(DelegatedStepEvent)`, where `DelegatedStepEvent` has:
    - `at_ms: u64`
    - `intent: String`, unsubstituted
    - `action: StepAction`
    - `candidates: Vec<CandidateSummary>`, which is `{id, role, label}`
    - `probabilities: BTreeMap<String, f64>`
    - `pick: Option<String>`
    - `outcome: DelegatedOutcome`, which is `Acted` or `HandedBack(HandBackReason)`
    - `threshold: f64`
    - `model: String`
    - `latency_ms: u64`
  - `pub async fn delegate_step(actor: &mut Actor, args: Value) -> Result<Value, SessionError>`
  - Tool arguments: `{ "intent": string, "action": StepAction, "text"?: string, "option"?: string, "window": <as get_window_state> }`

- [ ] **Step 1: Check the Cua contract.** In the 0.38.0 contract, check whether `type_text` and `set_value` take `element_token`. If `type_text` doesn't, a Type step clicks the token first and then calls `type_text` without a target. Record what you found in `docs/facts.md`.

- [ ] **Step 2: Write `FakeDecisionModel`.** It returns a queued `Result<Decision, DecisionModelError>` and records each `DecisionRequest`. A `never()` constructor gives a future that never resolves.

- [ ] **Step 3: Write the failing tests** in `delegate.rs`, using `FakeDriver` with a canned `get_window_state` reply (the Task 2 fixture) and `start_actor_with_credentials`:
  - `a_confident_step_clicks_the_candidates_element_token`: the driver sees `get_window_state` then `click` with `"element_token": "s00000008:39"`, and the result carries `pick`, `probability` and `decision_ms`.
  - `a_step_runs_get_window_state_once`
  - `a_handed_back_step_does_not_act_and_lists_the_top_candidates`: the driver sees only `get_window_state`, and the result text says "No action taken" and names up to five candidates with probabilities.
  - `a_secret_never_reaches_the_decision_model`: the intent and `text` contain `${TEST_PASSWORD}`, and no recorded request contains the secret value. The driver's `type_text` arguments do contain the substituted value.
  - `a_model_timeout_hands_back_within_the_limit`: `FakeDecisionModel::never()` with `timeout_ms = 200`, under paused time. The call returns `HandedBack(ModelFailed)`, and a following `get_window_state` call works.
  - `every_step_adds_one_delegated_step_event`: one for an acted step and one for a handed-back step. The event's `intent` holds `${TEST_PASSWORD}`, not the value.
  - `an_unknown_credential_name_is_refused_before_the_model_is_asked`
  - `the_report_counts_delegated_acted_and_handed_back_steps`: in `report.rs` tests.

- [ ] **Step 4: Run them to see them fail**
  Run: `cargo test --lib delegate`
  Expected: compile errors.

- [ ] **Step 5: Implement `session_delegate.rs`**
  1. Validate the credential names in `intent` and `text`.
  2. Call `call_driver` for `get_window_state`, and mask its result.
  3. Call `candidates`.
  4. Wrap `decide` in `tokio::time::timeout(timeout_ms)`.
  5. Call `decide_action`.
  6. On `Act`, build the Cua arguments with the element token and the unsubstituted text, then `Credentials::substitute`, then `call_driver`, then `mask_reply`.
  7. Push `DelegatedStep`, then return.

  - An acted step is an Action, counted the way `forward_call` counts one.
  - A handed-back step is a read-only call.
  - The handed-back text is: "No action taken: {reason}. Act on the window yourself. Top candidates: c1 button \"Sign in\" 0.41, ..."
  - `credential_scrub.rs` masks the intent and the candidate labels in the event.

- [ ] **Step 6: Run the gates**
  Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
  Expected: clean.

- [ ] **Step 7: Commit**
  `git commit -m "feat(delegate): carry out a delegated step in the session"`

### Task 6: List the tool, guide the Verifier, record the decision

**Files:**
- Modify:
  - `src/mcp.rs:39` (`own_tools`, or the forwarded schema list, whichever `call_tool` at :101-111 routes through to the actor; it must reach the session as `Request::CallTool`)
  - `src/mcp.rs:53` (`can_wait`)
  - `src/registry.rs:363` (`ListTools` adds `delegate_step` only when `decision_model` is set)
  - `src/daemon_start.rs` (build `JevClient` from config. A `MissingKey` error is a daemon log line, and the tool stays unlisted.)
  - `src/verifier_protocol.rs:4` and `docs/design.md:103`, kept in step
  - `docs/adr/0010-a-fresh-verifier-checks-each-change.md`
  - `docs/GLOSSARY.md`, `docs/design.md`, `README.md`, `CHANGELOG.md`

- [ ] **Step 1: Write the failing tests**
  - `delegate_step_is_listed_only_when_a_decision_model_is_configured`: in the registry tests, with both configs.
  - `the_verifier_protocol_mentions_delegate_step`: or extend the test that keeps `VERIFIER_PROTOCOL` in step with `design.md`, if there is one.
  - `keek_status_shows_the_decision_model_but_never_its_key`: it shows the endpoint, model and threshold. The spec's `get_config` is Cua's tool, so keek's own status carries these.

- [ ] **Step 2: Run them to see them fail**
  Run: `cargo test --lib -- registry verifier_protocol keek_status`
  Expected: fail.

- [ ] **Step 3: Implement and write the docs**
  - **Protocol text:** "When a decision model is configured, use `delegate_step` for a routine step whose target is clear from its words, such as a named button, field or link. Look at the screen yourself before you mark a Check. After 'No action taken', act yourself."
  - **ADR 0010:** an amendment saying keek may hold a Decision Model key, read only from an environment variable, and why.
  - **GLOSSARY:** Decision Model, Delegated Step.
  - **README:** the `[decision_model]` table. The variable named by `api_key_env` must be set where the keek daemon starts, which is the coding agent's environment. Screen state goes to the configured endpoint.
  - **CHANGELOG, Added:** "The Verifier can hand routine steps to a fast decision model, such as TypeSafe's Jev, and keek acts only when the model is confident."

- [ ] **Step 4: Run the gates**
  Run: `cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test`
  Expected: clean.

- [ ] **Step 5: Commit**
  `git commit -m "feat(delegate): offer delegate_step when a decision model is set"`. Commit the docs on their own as `docs(adr): let keek hold a decision model key`.

### Task 7: Offline replay and the timing script

**Files:**
- Create: `examples/delegate_replay.rs`, `examples/session_timing.rs`, `src/step_candidates_tree.rs` (and its tests)

**Interfaces:**
- Produces:
  - `pub fn candidates_from_tree(tree_markdown: &str, action: StepAction) -> Vec<(usize, Candidate)>`. It parses `[index] role "label"` lines in Cua's `tree_markdown`, read from a filed `before_state.json`.
  - `cargo run --example delegate_replay -- <evidence-root> [--thresholds 0.7,0.8,0.9,0.95] [--limit N]`
  - `cargo run --example session_timing -- <evidence-root> [--compare <evidence-root>]`

- [ ] **Step 1: Write the failing tests** for `candidates_from_tree`. Use a short `tree_markdown` taken from one filed `before_state.json` and scrubbed of anything personal.
  - `tree_lines_become_candidates_keyed_by_element_index`
  - `lines_without_a_label_are_skipped_for_click`

- [ ] **Step 2: Run them to see them fail**
  Run: `cargo test --lib step_candidates_tree`
  Expected: fail.

- [ ] **Step 3: Implement the parser and the replay example**
  - The replay walks `*_actions/*/` folders and keeps clicks whose arguments carry `element_token`. It takes the index after `:` in the token.
  - It builds candidates from the tree, writes the intent as "click {label}", and calls `JevClient`.
  - It prints top-1 accuracy, the abstention rate and p50/p95 latency per threshold.
  - Before the first request it prints the endpoint and the number of trees it will send, then waits for `y` on stdin.

- [ ] **Step 4: Implement `session_timing`.** It reproduces the spec's figures from `*_timeline.json`:
  - share of active time between calls
  - median gap and median call
  - share of `get_window_state` calls

  It leaves out Help Request and Takeover holds and gaps over 2 minutes. With `--compare`, it prints both sets side by side.

- [ ] **Step 5: Check the timing script against the spec.** Run `cargo run --example session_timing -- ~/Videos/keek`. Expected: about 77%, 1.9 s, 0.29 s and 46%, as in the spec's Why section. Report any difference rather than tuning toward it.

- [ ] **Step 6: Don't run the replay against the hosted endpoint.** Ask the developer first, as the spec requires. If they say yes, run it with `--limit 50` and record the result in the PR.

- [ ] **Step 7: Commit**
  `git commit -m "feat(delegate): add an offline replay and a session timing report"`

## Delivery

One pull request from `feat/delegated-steps`, titled `feat(delegate): hand routine steps to a decision model`, through pr-flight. The body:
- says why `reqwest` was added;
- links the spec and the ADR amendment;
- lists each gate's output and the `session_timing` output;
- gives the replay result, or "not run: waiting for the developer's go-ahead to send past trees".

The three-and-three timing comparison runs after merge and is not a merge criterion.
