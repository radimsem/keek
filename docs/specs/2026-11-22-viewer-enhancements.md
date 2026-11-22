# Viewer enhancements: gallery, Step away, live Checks, window sizing

Design for four changes to the Agents Workspace, agreed with the developer on 2026-11-22. Terms follow [`GLOSSARY.md`](../GLOSSARY.md). The screens are drawn at 1:1 in the proposals canvas (https://claude.ai/artifact/JzPWohPGuYshxKJPGUfit9), which reuses the parts of the approved comps.

## Why

The developer ran keek for several days on real work and hit four problems:

1. Check ticks in the Session Footer fill in all at once at the end, so the footer says nothing about progress while the Verifier works.
2. A Takeover cannot be left. Fetching a secret from the host password manager means pressing Done first, and the Verifier then treats the Help Request as answered and gives up.
3. Only two sessions are visible. A third or fourth open session has no place on the Agents Workspace.
4. A Session Window opens at 1280x800 inside a much larger slot, on black, and the Verdict panel overhangs it.

## Goals

- The developer can tell from the footer which Checks are done while the session runs.
- The developer can leave a Takeover, use their own desktop, come back and carry on. Only Done hands control back.
- A secret copied on the host can be pasted into the session during a Takeover.
- Up to four sessions are visible and reachable on the Agents Workspace.
- A Session Window fills its slot unless the Project Profile says otherwise.

## Non-goals

- Watching more than four sessions. The cap stays configurable; the layout is designed for four.
- Live moving thumbnails in the rail.
- Copying anything out of the session to the host clipboard.
- Scaling a Session Window. A window is always shown at its true size.

## Anti-goals

- A refused tool call because of when a Check was marked. The Verifier is nudged, never blocked.
- A secret pasted during a Takeover reaching the Action Log, the Session Timeline, a process command line or the Verifier's `clipboard_read`.
- A host shortcut acting on the session, or a session key acting on the host, at any moment of a Takeover, paused or not.
- Disturbing the Raw Recording when sessions swap between the focus slot and the rail.

## Constraints

- A nested Hyprland's output takes the size of its window, and Hyprland cannot draw a window scaled ([ADR 0007](../adr/0007-nested-window-is-the-session-window.md)). A Session Window's size is fixed when the session opens.
- The sandbox has no host Wayland or D-Bus socket ([`facts.md`](../facts.md#takeover-clipboard-results)), so the host clipboard reaches it only if keek carries it in.
- keek does not see the developer's Takeover input: it arrives on the nested seat, not through Cua.
- `no_focus` from map time is what keeps host input out of a session outside a Takeover ([`viewer.md`](../viewer.md)).

## Approaches considered

| Area | Options | Picked | Why |
|---|---|---|---|
| Leaving a Takeover | Step away and return with paste; paste only; step away without clipboard | Step away and return, with paste | The developer leaves to fetch something to paste. Either half alone leaves the trip broken. |
| Clipboard reach | Host to session during a Takeover, cleared on Done; two way; host to session, kept after Done | Host to session, cleared on Done | Two way lets a page in the sandbox write the real clipboard. Keeping it lets `clipboard_read` return the secret. |
| Done with nothing changed | Tell the Verifier what happened; ask the developer on Done; keep as is | Tell the Verifier | No extra click, and the Verifier stops treating an empty Done as an answer. |
| Checks marked late | Remind in tool results; refuse bulk marking; stronger wording only | Remind in tool results | Wording alone is already ignored. Refusing blocks the honest case where one screen proves two Checks. |
| More than two sessions | Wide focus with a slim rail; focus with wide strip panels; filmstrip below; even 2x2 grid; full focus with tabs; carousel | Wide focus, slim rail | Wide panels left blank space. A grid makes every window and video small. Tabs and a carousel hide sessions that may be stuck. |
| Rail cards | Still, name, state and ticks, with the Check list on hover; always the full Check list; still and colour only | Still, name, state, ticks | The full list made the cards bulky. A still alone says too little. A live miniature needs a capture path nobody has proven. |
| Picking the focus | Click the card plus shortcuts; click only; Show and Take control buttons per card | Click plus shortcuts | Buttons on every card add clutter. |
| Default cap | Keep 2; raise to 4; keep 2 and always use the focus layout | Raise to 4 | The developer wants four in parallel out of the box. |
| Default window size | Fill the slot; keep 1280x800 | Fill the slot, no cap | The black frame reads as a bug. The video then matches what was on screen. |
| Profile window smaller than slot | Centred with panels clamped; scale up; leave as is | Centred, panels clamped | True size is the point of a profile `window`. Scaling is unproven. |

## Design

### 1. Step away during a Takeover

- The `keek` submap gains a second bind, Super+Tab, meaning Step away. The footer shows a **Step away** button beside **Done** during a Takeover. *(my call: the key)*
- Step away pauses the Takeover: keek resets the submap, sets `no_focus` again and returns focus to the developer's previous window. The Verifier's held call stays held. The idle timeout stays paused and the recording keepalive keeps running, as during a Takeover.
- While paused, the Session Card has a yellow border, the footer reads "Takeover paused" with the time away, and offers **Resume** and **Done**. A panel over the Session Window says to click the session to carry on. keek sends one desktop notification, "Takeover paused · <project>", dismissed on Resume or Done. *(my call: yellow, the notification)*
- Resume, from the button or a click on the card body, does what Take control does: clear `no_focus`, focus the Session Window, enter the submap.
- Done works from both the running and the paused state.
- The Session Timeline keeps one Takeover entry from Take control to Done, and records each paused span inside it. *(my call)*
- The developer's submap snippet in the docs gains the new bind. Leaving the `keek` submap already means Done, so Step away needs its own signal, such as a second submap name; the shape is the builder's. keek detects a snippet without the bind through `hyprctl binds -j`, and the footer then says the shortcut is missing. The Step away button works either way. *(my call)*
- One Takeover at a time. While a Takeover is running or paused, its session stays in the focus slot, and Show and Take control on every other card are disabled until Done.

### 2. Paste from the host during a Takeover

- At Take control and at every Resume, keek copies the host clipboard's plain text into the nested clipboard. Other types are skipped.
- The text travels through connect-exec on standard input or a file descriptor, never on a command line and never through a Cua call, so it enters neither the Action Log nor the Session Timeline.
- On Done keek clears the nested clipboard before releasing the held call. If the nested Omarchy shell keeps a clipboard history, keek clears that too or keeps the carried text out of it; the builder checks which applies and records it in `facts.md`.
- connect-exec is a one-shot launcher today and pasta closes descriptors above stderr, so this needs a new way to pass the text in. That is part of the ticket.
- Nothing is copied from the session to the host.
- What the developer pastes stays visible on screen and in the video, as typed text does today. The docs keep saying so.
- This changes the documented rule that the nested desktop does not bind the host clipboard. `viewer.md` and `facts.md` are updated, and [ADR 0006](../adr/0006-nested-hyprland-in-a-sandbox-under-your-uid.md) if it states the rule.

### 3. An honest hand-back

- keek compares the session's screen at Take control with the screen at Done. *(my call: the comparison is how "nothing changed" is decided, because keek cannot see the developer's input)*
- If the screen differs, the hand-back note stays as it is: the Help Request is answered. A carried clipboard alone does not count, because the host clipboard nearly always holds text. *(my call)*
- If the screen looks the same, keek still releases the held call. The note says the developer pressed Done and the screen looks unchanged, that the Help Request may not be answered, and that the Verifier should re-read the screen and either call `request_help` again or skip the Check with a comment.
- The Session Timeline gains a third Help Request outcome, unanswered, beside answered and withdrawn. The Verification Video treats an unanswered wait like an answered one. *(my call)*
- The footer shows which of the two happened.

### 4. Checks marked as the Verifier goes

- keek counts forwarded Cua calls since the last `mark_check` or `add_check`. When the count reaches 8 and a Check is unmarked, keek appends one line to the next tool result naming the unmarked Checks and asking the Verifier to mark what it has already observed. The count then restarts. *(my call: 8)*
- No call is refused or delayed.
- The reminder is recorded in the Session Timeline so a report reader can see it happened. The footer's state column shows "Reminded the Verifier to mark Checks" until the next call.
- The footer rings the first unmarked Check as the one in progress. A rail card rings the same tick, and its hover list tags that Check "checking".

### 5. Wide focus, slim rail

- `session_cap` defaults to 4.
- The Agents Workspace has one focus slot and a rail. On a monitor at least 1920 logical pixels wide the focus slot takes the left of the workspace and the rail is a narrow column on its right, about a sixth of the width. Narrower monitors put the focus slot on top and the rail below it as a row. *(my call: the proportions and the narrow layout)*
- With one session open there is no rail and the focus slot spans the workspace. With two or more, the focus slot keeps its size and the rail appears.
- A rail card is small: a still of its session, then the Project, branch and elapsed time, one line of state (the last Action, or "Needs you" and the reason), and the tick squares with the count. The still carries its age ("Still · 3 s ago") and refreshes every few seconds.
- Hovering a rail card shows its goal and the Check list with each Check's text and outcome.
- Clicking a rail card swaps that session into the focus slot and the focused one into the rail. A rail card has no buttons. Take control is on the focused footer only.
- Super+1 to Super+4 show a session directly while the `keek` workspace is active. The developer adds these binds to their Hyprland config with the submap snippet; keek ships none. *(my call: opt-in, as the workspace bind is today)*
- A session with a Help Request turns its rail card orange. It does not swap in by itself; activating its notification switches to the workspace and shows it.
- While a Takeover is running or paused, the rail cards are dimmed and do nothing.
- A session in the rail keeps rendering and recording at full size while out of sight. How its Session Window is kept off screen, and how stills are captured without entering the Action Log or the Session Timeline, is left to the builder and needs a short spike recorded in `facts.md`.
- An ended session keeps its place until dismissed and does not count against the cap, as today. In the rail its card shows the Verdict; clicking it brings the full end card to focus.
- A new session takes an empty place, the focus slot first. With no empty place it replaces the oldest ended card. *(my call)*
- When the focus slot empties and the rail holds a running session, the oldest one moves into focus. *(my call)*
- This reverses the non-goal in `viewer.md` about watching more than a couple of sessions, and replaces the two-slot layout there and in `DESIGN.md`.

### 6. Session Window size

- A Project Profile with no `window` gets a Session Window that fills the focus slot above the footer. A profile `window` is honoured as today.
- Every session opens at the focus slot's size, whichever slot it starts in, so it fits when swapped in.
- A window smaller than the focus slot stays centred on black. The Verdict panel, the rendering panel and the paused panel are no wider than the window.
- `design.md` and the profile docs replace "1280x800 without it" with the new rule, and say that the Verification Video's size then follows the developer's monitor.

## Build order

1. Session Window size (6). Small, and the gallery builds on it.
2. Checks reminder and the ringed tick (4).
3. Step away, paste and the hand-back note (1, 2, 3). One ticket each, in that order.
4. Wide focus and rail (5), after its spike.

## Left to the builder

- Module and flag names.
- The exact wording of notes and reminders, in the voice of `PRODUCT.md`.
- How screens are compared for the hand-back note, and what counts as the same.
- The still refresh interval and capture path.
- Colours for a third and fourth session.

## Docs to update with the code

`viewer.md`, `design.md`, `DESIGN.md`, `facts.md`, `GLOSSARY.md` (focus slot, rail, Step away), ADR 0007 and ADR 0006 where their text changes, and `CHANGELOG.md` per ticket.
