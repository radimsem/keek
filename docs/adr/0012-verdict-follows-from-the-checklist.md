# The Verdict follows from the Verification Checklist

A free-form pass or fail tells the coding agent little about what to fix, and a Verifier can pass a change it only half checked. So every Agent Session verifies a Verification Checklist: the acceptance checks the coding agent wrote with the goal, plus any the Verifier adds while testing. The Verifier marks each Check passed, failed or skipped, with a one-line comment for a failure or skip, and keek derives the Verdict: Pass when every Check passed, Needs fixes when any failed, Partly verified when none failed and some were skipped. A failed outcome reads "Needs fixes" because its reader is usually the coding agent, which fixes the change and runs another verification.

## Considered Options

- The Verifier picks pass, fail or inconclusive and lists its steps: simplest, but nothing stops a pass over a skipped criterion, and the steps don't map to what was asked.
- The Verifier writes the whole checklist from a free-text goal: the coding agent can't shape the test, but each run can split the same goal differently, so attempts don't compare.
- A skip counts toward Pass: fewer false alarms, but a skipped criterion would hide inside a Pass.

## Consequences

- `close_session` refuses while any Check is unmarked, so every Verdict accounts for each criterion.
- The coding agent writes the checks, so it still shapes what is verified; the Verifier can add Checks but not drop them.
- Attempts on the same change share their checks, so the Session Card and `index.md` can show "attempt 2 of 3" against comparable lists.
