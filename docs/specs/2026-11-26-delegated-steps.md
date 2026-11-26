# Delegated Steps: a fast decision model carries out routine steps for the Verifier

Design agreed with the developer on 2026-11-26. Terms follow [`GLOSSARY.md`](../GLOSSARY.md); new ones are proposed at the end. Facts about TypeSafe and decision models were checked on 2026-11-26 and carry their sources.

## Why

Verification is slow because the Verifier thinks between calls, not because keek is slow. Across the 142 filed sessions in `~/Videos/keek` (6,029 calls, 2,921 Actions), 77% of active time was the gap between one call's end and the next call's start. The median gap was 1.9 s, against a median call of 0.29 s. 46% of all calls were `get_window_state`, the Verifier looking before it acts. These figures leave out Help Request and Takeover holds and gaps over 2 minutes.

Decision models ("System One" models) answer a typed question about a state in about 0.1 s: pick one option, give a yes/no probability, or a score, each with probabilities. TypeSafe's Jev 1.14 takes 92 ms per task on the Artificial Analysis Decision Index v2 (https://artificialanalysis.ai/articles/decision-index-v2). Picking the control that matches "click Sign in" from a list of on-screen candidates is that kind of question. The developer wants the Verifier to hand such routine steps to a decision model, keep judging the outcome itself, and run that model on their own DGX Spark where possible.

## Goals

- The Verifier can hand keek a short intent, such as "click Sign in" or "type `${TEST_EMAIL}` into the email field". keek finds the control and acts in one call, without the Verifier reading the screen first.
- keek acts only when the decision model is confident. Otherwise it hands the Verifier the best candidates and does nothing.
- keek talks to hosted Jev or any server that speaks Jev's API, so a local model on the developer's DGX Spark works the same way.
- The Evidence shows every Delegated Step: the intent, the candidates, the probabilities, what keek did and how long the decision took.
- Off until the developer configures a decision model. Without one, keek behaves as today.
- Sessions with Delegated Steps take clearly less wall time for the same Verification Checklist, with no more failed or skipped Checks.

## Non-goals

- The decision model running a whole Check, or the whole Verification Checklist, by itself.
- Decisions from screenshots alone. v1 picks from the accessibility tree and the browser's page structure. Jev's image input is still marked preview quality (https://docs.typesafe.ai/images.md).
- One adapter per vendor. keek speaks Jev's API only, and other models run behind a Jev-compatible server.
- Choosing, hosting or benchmarking models for the developer, beyond the offline replay below.
- Running openjev or another server on the DGX Spark. That is the developer's setup, outside keek.

## Anti-goals

- A decision model marking a Check, or a Verdict that rests on its judgement. The Verifier observes the result and marks every Check, as today ([ADR 0012](../adr/0012-verdict-follows-from-the-checklist.md)).
- keek acting on a low-confidence pick, or on a pick outside the candidates it offered.
- A Test Credential's secret value reaching the decision model. The intent carries `${NAME}` placeholders, and keek substitutes values only after the decision, when it acts. The screen state keek sends is scrubbed with the same masking as tool results.
- Screen data leaving the machine without the developer saying so. A hosted endpoint is an explicit setting, and its docs say the accessibility tree goes to that service.
- A Delegated Step hiding from the Evidence, or keek retrying one on its own.

## Constraints

- **A fresh Verifier.** The coding agent never drives a session ([ADR 0010](../adr/0010-a-fresh-verifier-checks-each-change.md)). The decision model works inside the Verifier's session, called by the Verifier, so this holds.
- **keek holds a model credential.** ADR 0010 rejected keek having model credentials of its own. A hosted endpoint needs an API key, so the implementation PR amends ADR 0010. The key stays in an environment variable keek reads, never in config, output or Evidence. A local server needs no key.
- **MCP server only.** keek can't choose the Verifier's model or limit its tools ([ADR 0011](../adr/0011-mcp-server-only.md)). The new tool and when to use it are described in the server's instructions.
- **Input goes through Cua.** Cua Driver owns input and recording ([ADR 0003](../adr/0003-cua-driver-owns-input-and-recording.md)), so keek performs the chosen action with Cua's own tools, using element tokens from the same `get_window_state` capture the candidates came from.
- **Shared pieces go to Cua.** ([ADR 0009](../adr/0009-shared-pieces-go-to-cua-driver.md)) Cua already has this loop as an example, `libs/cua-driver/examples/jev-use`. Cua Driver observes, Jev picks one of at most 24 candidates plus `reobserve` and `abstain`, and the caller checks the result independently. It covers Linux AT-SPI and the browser page structure. Where the loop lives is the one open question below.
- **Testable without a desktop.** The decision model is an injected trait, so `cargo test` uses a fake. Live tests are `live_*` and `#[ignore]`.
- **Jev's API** (https://docs.typesafe.ai/api.md): `POST https://api.typesafe.ai/v1/systemone` with a Bearer key. The body is `{model, state, questions}` and the answer gives per-option probabilities and a confidence. Choice takes up to 255 options. A request holds 64k tokens, and independent questions in one request run in parallel.
- **Jev is hosted only.** TypeSafe publishes no weights (https://typesafe.ai/enterprise). Jev-compatible servers that run locally include openjev (https://github.com/razorback16/openjev, Apache-2.0, vLLM, NVIDIA GPU with 24 GB or more) and local-jev. Neither states support for the DGX Spark's ARM64 GB10.
- **Benchmarks don't cover this task.** The Decision Index tests knowledge, multiple-choice reasoning, tool calling, safety classification, document extraction and answer consistency, not picking a control on screen. Open-weights models listed by a third-party board (https://systemonemodels.org/compare/best-ai-decision-models/) include GLiDE 27B, Kev 27B and H2O-Lightning-31B; whether each speaks Jev's API was not checked.

## Approaches

| Approach | Trade-off | Outcome |
| --- | --- | --- |
| Personal setup: run Cua's jev-use loop beside keek | No keek change, but it can't reach the sandboxed driver except through keek's tools, and its decisions miss the Evidence and credential masking. | Rejected |
| The Verifier hands keek single steps; keek asks the decision model and acts if confident | Keeps the Verifier as judge and every rule above. The gain is limited to routine steps. | **Chosen** |
| The decision model runs the whole Verification Checklist; Claude reviews the Evidence after | Fastest, but a small model would effectively decide the Verdict from accessibility data. | Rejected |

For model access, the developer chose Jev's API only, over one adapter per vendor or an OpenAI-style fallback without probabilities.

## Design

### The tool

A new forwarded tool, provisionally `delegate_step`, listed only when a decision model is configured. Its arguments:

- `intent`: what to do, in plain words.
- `action`: click, double click, right click, type or select. Typing and selecting take `text` or an option, which may hold `${NAME}`.
- `window`: the target, as other tools take it.

keek then:

1. Calls `get_window_state` once, for the tree and the capture together.
2. Builds the candidates the action can apply to from that AT-SPI tree, web content included: role, name, value, bounds and an id that maps back to the element token. *(my call)* 2,692 of the 2,779 filed `before_state.json` trees are Chromium pages read through AT-SPI, and Project Profiles start their own Chromium, so the `get_browser_state` path, which needs a browser opened with `browser_prepare`, would miss nearly every real session. This is where keek departs from jev-use, whose desktop path drops `in_web_content` elements. Like jev-use, it caps them at a small number (24 in that recipe) and adds `reobserve` and `abstain`.
3. Scrubs secret values from the state and sends one request with two questions over the same state: a Choice of candidate, and a Noul for "one of these candidates is what the intent means".
4. Acts only if the top candidate's probability and the Noul both clear the configured threshold, and the pick is one of the offered ids. keek substitutes `${NAME}` values, then calls Cua's `click`, `type_text` or `set_value` with the element token.
5. Returns what the matching Cua tool returns, plus the pick, its probability and the decision time. On `abstain`, `reobserve` or low confidence, it returns no action, the top candidates with their probabilities, and a line telling the Verifier to act itself.

One `delegate_step` call is one Action. A step that didn't act is a read-only call.

### Configuration

A `[decision_model]` table in keek's config (`~/.config/keek/config.toml`):

- `endpoint`: the API base URL. TypeSafe's, or a local server.
- `model`: such as `jev-latest`.
- `api_key_env`: the name of the environment variable that holds the key, empty for a local server.
- `threshold`: the probability needed to act.
- `timeout_ms`: how long to wait for a decision.

`get_config` shows the endpoint, model and threshold, never the key. *(As built, `keek_status` shows them instead, because `get_config` is Cua Driver's own tool, forwarded unchanged.)* An unreachable endpoint or a timeout returns the candidates, as a low-confidence pick does, and the session goes on.

### Verifier guidance

The server instructions say to use `delegate_step` for routine steps whose target is clear from the intent, such as named buttons, fields and links. The Verifier still looks at the screen before marking a Check, and acts itself after an abstention.

### Evidence

- Every Delegated Step, acted or handed back, adds a `delegated_step` event to the Session Timeline: the intent, candidates, probabilities, pick, outcome, threshold, model and latency. *(my call)* Cua makes a `turn-*` folder only for an Action, so a handed-back step has none, and filing already scrubs `timeline.json`.
- The Verdict Report gives the counts: steps delegated, acted and handed back.

### Offline replay

Before the threshold is trusted, a replay tool runs a configured model over filed Evidence. Filed Evidence holds Cua's `before_state.json` per Action but no `get_window_state` results, and most clicks are by pixel. So the replay uses only clicks by element token, about 12% of a 400-click sample, whose index names the element in `before_state.json`. *(my call)* For each, it rebuilds the candidates from that tree and asks for the control that was used. Once Delegated Steps are in use, their timeline events become better replay data. It reports top-1 accuracy, the abstention rate and latency at several thresholds.

The Action Log has no Verifier intent, so the replay writes one from the element's role and name ("click Sign in"). That makes its accuracy an upper bound. It is a developer command or test helper, not a Verifier tool.

*(As built.)* With the label as the intent, every threshold from 0.7 to 0.95 scored the same, so the replay writes the intent three ways, chosen with `--modes`:

- `label`: "click Sign in", the upper bound above.
- `partial`: the role and the label's longest word, "click the sign button", so controls that share the word compete.
- `absent`: a label from another tree that this screen holds nowhere. No offered control is meant, so the right outcome is a hand-back.

The run on 2026-11-26 sent 80 trees in each mode to hosted `jev-latest` (240 requests, none failed, p50 158 ms, p95 190 ms). Right and wrong count acts. A wrong act in `partial` is often a fair pick of another control with the same word, and the replay lists every wrong act so they can be judged.

| Threshold | `label` right | `partial` right, wrong | `absent` wrong | All handed back |
| --- | --- | --- | --- | --- |
| 0.7 | 79/80 | 70, 3 | 4 | 35.0% |
| 0.8 | 79/80 | 68, 2 | 0 | 37.9% |
| 0.9 | 78/80 | 63, 1 | 0 | 40.8% |
| 0.95 | 77/80 | 58, 1 | 0 | 43.3% |

The default stays 0.9. Every `absent` act had a Noul of 0.75 or less, so any threshold above 0.75 stops them all, and 0.9 keeps a margin above that. The one wrong act left at 0.9 picked a link labelled exactly as the intent's word, with a probability of 1.0 and a Noul of 0.99, which no threshold up to 0.99 stops. Going to 0.95 hands back 6 more right picks and stops no wrong one.

### Measuring the gain

The same Verification Checklist on the same Project runs with and without a decision model, three times each. The comparison is active time, calls, `get_window_state` calls and Check outcomes, from the Session Timelines. The analysis that produced the figures under Why is kept as a reusable script.

## Decided after review

- **The loop lives in keek first**, behind the injected trait. The reusable part is offered to Cua once it has proven itself, and nothing is posted to `trycua/cua` until the developer approves the text. Cua's jev-use drops web content, which is nearly all of keek's work, so keek's loop would differ from Cua's anyway.
- **Hosted Jev is the endpoint to build and test against.** The developer has a TypeSafe API key, in `~/.config/typesafeai/` or the GNOME keyring. keek reads it only through `api_key_env`, and it never enters output, config, Evidence or commits. A local Jev-compatible server on the DGX Spark comes later, outside keek.
- **Sending past-session trees to hosted Jev** in the offline replay was not answered directly. Ask the developer before the first replay run against the hosted endpoint.
- **The timing study is not a merge criterion.** *(my call)* The PR ships the tool, the replay and the analysis script. The three-and-three comparison runs afterwards, and its result goes on the PR or a follow-up.

## Left to the builder

- The tool's final name, checked against the glossary.
- The candidate cap, and how candidates from the browser page structure and from AT-SPI are merged.
- The question wording, and the default threshold, taken from the offline replay.
- Whether a Delegated Step's result includes a compact fresh window state, to save the Verifier its next `get_window_state`.

## Proposed glossary terms

- **Decision Model**: a model that answers a typed question with a probability for each option, such as TypeSafe's Jev.
- **Delegated Step**: one routine step the Verifier hands to keek, which a Decision Model resolves to a control before keek acts.
