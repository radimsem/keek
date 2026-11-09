# keek

keek lets a coding agent have its change verified by a Verifier that uses the app the way a person would, on an isolated Omarchy desktop, and hands the reviewer an edited video of what it did.

## Session

**Agent Session**:
One isolated, recorded desktop run in which a Verifier checks a change, from open to close. Each git worktree has at most one open Agent Session.
_Avoid_: workspace, sandbox, VM, run

**Incomplete Session**:
An Agent Session that ended through a crash, an exit, an idle timeout, a window it opened on the developer's desktop besides its Session Window, or a close that could not stop the recording. It still produces Evidence but has no Verdict.
_Avoid_: failed session, aborted run

**Verifier**:
The agent that drives an Agent Session and reaches its Verdict. The coding agent starts a fresh one for each verification, so a Verdict never comes from the agent that wrote the change.
_Avoid_: tester, QA agent, checker

**App Under Test**:
The application from the project's worktree that the Verifier starts and exercises inside an Agent Session.
_Avoid_: target, SUT

## Project

**Project**:
The app whose Agent Sessions keek groups together, named by the coding agent for each verification. Its Agent Sessions can come from any worktree of its repository, and one repository can hold several Projects.
_Avoid_: repo, workspace

**Project Profile**:
How keek builds, starts and reaches a Project's App Under Test on one machine: its install and start commands, pinned toolchains, environment, forwarded ports and window size. A coding agent writes it; the developer rarely edits it by hand.
_Avoid_: settings, config, recipe, Setup Script

**Project Layer**:
A Project's installed dependencies, built once by its Project Profile's install command and reused across Agent Sessions until the profile or the lockfiles change.
_Avoid_: image, snapshot, cache

**Toolchain Store**:
The toolchains keek installs on the host for Project Profiles that pin a version, shared read-only by every Agent Session.
_Avoid_: toolchain cache, runtime

**Test Credentials**:
Smoke-test-only accounts and keys that a project grants to its Agent Sessions, kept outside the project and chosen so they are safe to appear in Evidence. A Verifier refers to them by name and never sees a secret one's value. A public one, such as a test username, may appear in Evidence.
_Avoid_: secrets, env, .env

**Test Files**:
Sample inputs a Project grants to its Agent Sessions, such as a document to upload. They are kept outside the repository, placed there by hand, and chosen so they are safe to appear in Evidence.
_Avoid_: fixtures, samples, shared files

## Recording

**Action**:
One input the Verifier applies to the desktop, such as a click, typed text, a key press or a scroll. Reading the screen or its accessibility tree is not an Action.
_Avoid_: step, event, turn

**Action Log**:
The record of every Action in an Agent Session and what it returned.
_Avoid_: trajectory, trace, history

**Session Timeline**:
The timed record of an Agent Session's calls, Actions, Check marks, Help Requests and Takeovers, counted from the start of its Raw Recording. Times in the Verification Video and Verdict Report come from it.
_Avoid_: action history, activity graph, trace

**Raw Recording**:
The uncut video of a whole Agent Session.
_Avoid_: capture, footage

**Idle Stretch**:
A span with no Action nearby in which the screen is also still.
_Avoid_: thinking time, dead time, gap

**Fast-forward**:
An Idle Stretch shown compressed in the Verification Video.
_Avoid_: cut, trim, skip

**Verification Video**:
The edited video a reviewer watches: every Action with its effect at real speed, each Idle Stretch as a Fast-forward, and each Takeover as a short blurred stretch. A Help Request's wait is cut.
_Avoid_: final cut, reel, recording

## Watching

**Agents Workspace**:
The one Hyprland workspace that holds every Session Card.
_Avoid_: agent desktop, grid, dashboard

**Session Window**:
The live window of one Agent Session's desktop on the developer's screen. It ignores the developer's input except during a Takeover.
_Avoid_: viewer, tile, preview, nested window

**Session Footer**:
keek's bar joined to the bottom of a Session Window. It shows the Project, branch, goal, last Action, elapsed time and any Help Request, and holds the control to take the session over.
_Avoid_: Agents Strip, status bar, panel

**Session Card**:
A Session Window and its Session Footer, seen as one card in a slot of the Agents Workspace. After the Agent Session ends, the card keeps its last frame and shows the Verdict until the developer dismisses it.
_Avoid_: tile, panel, dashboard

**Help Request**:
A Verifier's signal that it cannot continue without the developer, with a short reason. It is open from when the Verifier raises it until the developer takes control or the Verifier withdraws it.
_Avoid_: needs-you, escalation, blocker

**Takeover**:
A stretch in which the developer, not the agent, drives an Agent Session, from Take control to Done. The Verifier waits for its whole length.
_Avoid_: handoff, intervention, manual mode

## Outcome

**Verification Checklist**:
The Checks an Agent Session verifies: the acceptance checks the coding agent wrote, plus any the Verifier adds while testing.
_Avoid_: test plan, steps, todo list

**Check**:
One observable acceptance criterion on a Verification Checklist. The Verifier marks it passed, failed or skipped; a failed or skipped Check carries a short comment saying what it saw or why it could not check.
_Avoid_: test, step, assertion

**Verdict**:
The outcome of an Agent Session, which follows from its Verification Checklist: Pass when every Check passed, Needs fixes when any Check failed, and Partly verified when none failed but some were skipped. It is final; a different outcome needs a new Agent Session.
_Avoid_: result, status, fail

**Verdict Report**:
The Verifier's written account of an Agent Session: the goal, the branch and commit it verified, the Verification Checklist with each Check's outcome, comment and time in the Verification Video, and the Verdict.
_Avoid_: summary, test report

**Evidence**:
Everything an Agent Session leaves for review: the Verification Video, the Raw Recording, the Action Log, the Session Timeline and the Verdict Report.
_Avoid_: artifacts, outputs
