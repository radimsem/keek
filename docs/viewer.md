# keek viewer design

How the developer watches Agent Sessions and takes them over on their Omarchy desktop. Terms are in [`GLOSSARY.md`](GLOSSARY.md); the session, recording and Evidence design is in [`design.md`](design.md).

## Intent

The developer codes while up to four agents verify changes in the background. Every so often they glance at the agents to answer one question: is each one making progress, or stuck? When an agent hits something it cannot get past (a 2FA prompt, a captcha, an OS dialog), the developer takes over that one session briefly, then hands it back and the agent carries on.

The model is the watch, take control, done pattern of Grok Bot, Manus and Anthropic's computer-use demo. Because each Agent Session is a nested Hyprland, its desktop is already a live window on the developer's screen, and keek only adds what that window lacks ([ADR 0007](adr/0007-nested-window-is-the-session-window.md)).

## Goals

- A glance tells the developer, per session: which Project and session it is, what it is verifying, what it did last and how long ago, and whether it needs them.
- A session that needs the developer gets their attention even when they are not looking at the agents.
- The developer can drive any session on demand, the agent waits meanwhile, and both sides know exactly when control changed hands.

## Non-goals

- Watching more than four sessions at once. keek caps open sessions at four (configurable). The layout is designed for four; a larger cap gets smaller rail cards, not a fleet view.
- Live moving pictures in the rail. A rail card shows a still a few seconds old.
- An Omarchy shell plugin or bar widget. The Session Footer is keek's own layer-shell surface.
- Remote watching from another machine or a phone.
- Chatting with the agent beyond starting and ending a Takeover.

## Anti-goals

The viewer fails, even if it works, if:

- It steals focus, pops over the developer's editor, or makes noise while nothing needs them.
- Clicking or typing into a Session Window reaches the agent's desktop outside a Takeover.
- The Verifier acts while the developer is in control.
- A host shortcut pressed during a Takeover acts on the developer's desktop instead of the session, for example closing the Session Window.
- An agent's desktop changes size mid-session.
- Anything the developer types during a Takeover (a 2FA code, a real password) reaches the Action Log or the Verification Video.
- It costs noticeable CPU while the developer is not looking at it.

## Constraints

- Omarchy is the only supported host. Use Omarchy's helpers where they exist (`omarchy-notification-send` for notifications).
- Long-running host code reacts to events instead of polling, following Omarchy's rule for background work. Hyprland's event socket reports windows and workspaces. One exception: while an Agent Session is open, the daemon lists host windows once a second, because Hyprland sends no event when a window's class changes.
- Every nested Hyprland window has class `aquamarine` and title `aquamarine - WAYLAND-1`, and neither can be set. Its output takes the window's size on every resize.
- `hyprctl setprop address:<addr> no_focus 1` makes Hyprland refuse focus to a window and skip it in pointer hit tests. Setting it does not take focus away from a window that already has it.
- A Hyprland window rule matches a window's identity and state (class, title, tag, workspace among others), never a pid; only a dispatcher selector takes `pid:...`. A rule therefore cannot tell two nested Hyprlands apart.

## Decisions

### Agents Workspace

- keek uses one named Hyprland workspace, `keek`, for every Session Card.
- The workspace has one focus slot and a rail (see [Focus slot and rail](#focus-slot-and-rail)). The focus slot shows one session as a Session Card; the rail shows every other open or ended session as a rail card. With one session there is no rail and the focus slot spans the workspace.
- Every Session Window opens at the focus slot's size, whichever place its session starts in, so it fits when it is swapped in. Without `window` in the Project Profile, the Session Window fills the focus slot above the Session Footer, so the Verification Video size follows the developer's monitor. The Project Profile can set a different size, such as a phone size for a mobile web app. A smaller Session Window is centred on black at its true size, and so is a focus-size window while the focus slot spans the workspace. The agent's desktop and the Verification Video keep the Session Window size.
- A Session Window opens there silently: it never takes focus or switches the developer's workspace. It closes when its session ends. Its window rule includes `render_unfocused`, so the session keeps rendering and recording while the workspace is hidden, and while it waits in the rail.
- keek ships no keybinding to reach the workspace. Its docs suggest one for the user's Hyprland config. *(my call)*

### Viewer process and narrow monitors

GTK runs on the daemon process main thread; the Tokio daemon runs on a worker thread. A bounded channel carries Session Footer lifecycle messages, and each footer awaits its snapshot watch channel on GLib. If GTK cannot initialise, the daemon continues without a viewer. Hyprland workspace events control visibility, and the elapsed timer exists only while at least one footer is visible.

On a monitor at least 1920 logical pixels wide the focus slot takes the left of the workspace and the rail is a column on its right, about a sixth of the width. A narrower monitor puts the focus slot on top and the rail below it as a row. The Session Window keeps its opening size throughout the session.

### Session Window

The nested Hyprland's own window. It shows the agent's desktop live, with Cua's agent cursor, its action marks and a ripple on each click in the picture.

keek sets `no_focus` on it from the moment it maps until a Take control. The developer's clicks, scrolls and keys never reach the agent's desktop by accident, and the window can't be dragged or resized out of its slot.

keek's host rule matches class alone, so a nested Hyprland the developer starts themselves while a session is open is caught by it too: it opens floated on the `keek` workspace with `no_focus`, and keek lifts neither while a session is open. keek places each Session Window by its client pid, so the developer's window normally stays where the rule left it; only when the pid cannot be resolved does keek take the first new `aquamarine` window, which can be the developer's. Moving `no_focus` and the placement to per-window props once the pid matches would leave a mapped window focusable and off `keek` for a moment, and the Takeover boundary needs `no_focus` from map time (ADR 0007), so the class rule stays.

When the last session closes, disabling the rule lifts `no_focus` but not the placement. keek then moves each `aquamarine` window still on `keek` or `keek-rail` (where a mismatched placement can leave one) to the active workspace and tiles it, without switching to it. It moves only a window whose client process is named `Hyprland`: a Session Window belongs to a keek process, so another daemon's Session Windows stay where they are. While `keek` itself is the active workspace the window stays floating there, since a tiled window would cover the Session Cards. A daemon that stops on SIGTERM with a session open also leaves it on `keek`, because keek moves windows only when a last session closes. A window left either way is moved the next time a last session closes while another workspace is active.

### Focus slot and rail

keek places each session when it opens. It takes an empty place, the focus slot first, then the rail from the top. With no empty place it replaces the ended card that ended first. Ended sessions keep their place until dismissed and don't count against `session_cap`. When the focus slot empties and the rail holds a running session, the one that opened first moves into focus; an ended card never moves in by itself. Rail cards close up from the top when one leaves. *(my call)*

A rail session's Session Window waits on the `keek-rail` workspace, which nobody views, with the same `no_focus` and `render_unfocused` rule, so it keeps rendering and recording at full size ([`facts.md`](facts.md#in-a-real-agent-session)). Showing it moves its window to the focus slot on `keek` and moves the focused one to `keek-rail`, with `follow = false`, so the developer's workspace and focus never change and both Raw Recordings stay continuous. A session that opens in the rail maps on `keek`, as every Session Window does, and keek moves it out of sight as soon as it places the window. If GTK cannot start, all Session Windows stay stacked on the Agents Workspace and keek takes no rail stills.

A rail card shows:

- A still of the Session Window with its age ("Still · 3 s ago"). keek takes one every 3 s with `grim -T` on the host, from the window's stable id, and keeps it beside the end-card frames, outside the session folder. It never goes through Cua Driver, so it is in neither the Action Log nor the Session Timeline. An ended card shows its last frame.
- The Project, branch and elapsed time; one state line (the last Action, "Needs you" and the reason, or the Verdict); "2 of 4" with the tick squares, where the first unmarked Check wears the same ring as in the footer; and the session's `Super` number.
- On hover, a panel to its left with the goal and each Check's text and outcome, the one being checked tagged "checking".

A rail card is one button and has no others. Clicking it swaps its session with the focused one; an ended card brings its full end card to focus. Take control and everything else the Session Footer offers, including ending a session by hand, are reached by showing the session first.

A Help Request turns its rail card orange. The session does not swap in by itself: activating the notification switches to the `keek` workspace and shows that session.

While a Takeover runs or is paused, its session stays in focus. The rail cards are dimmed and do nothing, and a line under the rail reads "Other sessions unlock when you press Done." Shortcuts and notifications don't swap either.

Super+1 to Super+4 show the matching session while `keek` is the active workspace, once the developer adds the binds below beside the `keek` submap; keek ships none. Each bind emits a Hyprland custom event, `keek-show <n>`, that keek reads from the event socket; on any other workspace it does what Omarchy's own Super+number does. The numbers follow the order sessions opened in and stay with a session while it moves between the focus slot and the rail. *(my call)*

```lua
for n = 1, 4 do
  local key = "SUPER + code:" .. tostring(n + 9)
  hl.unbind(key)
  hl.bind(key, function()
    local workspace = hl.get_active_workspace()
    if workspace and workspace.name == "keek" then
      hl.dispatch(hl.dsp.event("keek-show " .. n))
    else
      hl.dispatch(o.workspaces.focus(n))
    end
  end)
end
```

### Session Footer

A gtk4-layer-shell surface for the session in the focus slot, shown only while the `keek` workspace is active. keek follows Hyprland's workspace events to show and hide it.

Once the session ends, the footer moves to the bottom layer with the rest of the card, so a window the developer opens on `keek` covers the whole Session Card.

It sits flush under its Session Window and overlaps the window's bottom border by the Hyprland rounding (8 px), so window and footer read as one Session Card. The footer always has its slot's width and sits at the slot's bottom. Around a Session Window smaller than its slot, the window floats centred above the footer instead of resting on it, and the black backdrop draws the card's outer edge in the window's current border colour and joins the footer's side borders; the window keeps its own border as a frame. The footer draws its own side and bottom border in the window's current border colour and rounds only its bottom corners. The nested desktop is sized so the overlap hides nothing but the border. A 2 px top edge in the session's cursor colour ties the card to its Cua cursor, on screen and in the video. *(my call)*

It shows that session's:

- Project and branch (`myapp`, `fix/17-login-redirect`), after a small arrow in the session's cursor colour. The start time stays in the Evidence file names, not on screen.
- Goal.
- Last Action in plain words (`click "Sign in"`) and the time since it, counting up.
- The Verification Checklist's progress ("Checked 2 of 4", counting marked Checks) with one tick per Check: filled green for passed, filled red for failed, an outlined yellow dash for skipped, a dim outline for not yet marked. While the Verifier works, the first unmarked Check's tick wears a Harbor Blue ring and dot, so the footer shows which Check is being checked. The ring is absent during a Help Request or a Takeover and once the session ends.
- Elapsed time since the session opened, so a stuck session stands out. Once the session ends it reads "Ran" with the total.
- Help Request state: the whole card turns orange, the window border through a window rule and the footer and any black backdrop with it, and the footer shows the Verifier's reason.
- During a Takeover: "You are in control" and its running time, with the window border in the active colour. If the `keek` submap has no Step away bind, the detail line says so instead.
- While a Takeover is paused: "Takeover paused" over the time away ("0:41 away · your shortcuts work again"), and the whole card turns yellow.
- After Done: "Back to the Verifier" over "Handed back after 1:12. Help Request answered." Without a Help Request behind the Takeover it reads "Handed back after 1:12." When the screen looks unchanged it reads "The screen looks unchanged. The Verifier was told."
- The **Take control** button. During a Takeover it gives way to **Step away** and **Done**, each after its shortcut, and while the Takeover is paused to **Resume** and **Done**.

When keek reminds the Verifier to mark Checks (see [`design.md`](design.md#agent-interface)), the state column reads "Reminded the Verifier to mark Checks" over "8 actions since the last mark" until the Verifier's next call. A Help Request, a Takeover and the view-only hint take precedence.

The keek daemon already sees every Action on its way to Cua Driver, so it pushes an update to the footer per Action and per state change; nothing polls. *(my call)*

### View-only hint

Outside a Takeover, `no_focus` makes Hyprland skip the Session Window in pointer hit tests, so the pointer lands on whatever is beneath it. keek puts a layer-shell surface on the bottom layer under each slot to catch that hover. It covers the whole card body, including the black sides around a Session Window smaller than its slot, which stay inert during a Takeover. While the pointer is over the card body, the footer's state column reads "View only" and "Use Take control to interact.", and the Take control button lights up. A click there does the same and nothing else. The footer shows the last Action again when the pointer leaves. While a Takeover is paused the hint stays off, and a click on the card body resumes the Takeover. *(my call)*

### Help Request

- The Verifier calls `request_help(session, reason)`. The call blocks until a Takeover on that session ends, sending progress notifications while it waits.
- That session's Session Card turns orange (the Omarchy needs-you colour) and its footer shows the reason. keek sends one critical desktop notification ("Agent needs you: 2FA code · myapp") through `omarchy-notification-send`, so it stays on screen until the developer acts, and dismisses it when the developer presses Take control. The shell dismisses by summary, so this also clears a second waiting session's notification; that session's card stays orange. Activating the notification switches to the `keek` workspace and shows that session in the focus slot, unless a Takeover holds the rail. *(my call)*
- If the agent cancels the `request_help` call before the developer presses Take control (Esc in the coding agent, or a client timeout such as Codex's 5 minutes of active time), keek withdraws the Help Request: the card and footer return to their earlier state, the orange border clears and the Verifier's next call runs at once. The notification stays. A Takeover already running when the cancel arrives continues until Done. Only `request_help` is withdrawn, not a forwarded Cua call or `close_session`.
- There is no timeout while a Help Request waits for the developer. The 5-minute idle timeout (see [`design.md`](design.md#abnormal-end)) pauses during a Help Request and a Takeover.

### Takeover

A Takeover starts only from the **Take control** button. Looking at, hovering over or clicking the Session Window never starts one.

1. Take control: keek holds the Verifier's next tool call (any forwarded Cua tool, or the pending `request_help`). Holding comes first, so no Verifier Action can land after the developer starts.
2. keek clears `no_focus`, focuses the Session Window and switches the host Hyprland into the `keek` submap. That submap binds only Super+Esc, which means Done, and Super+Tab, which means Step away, so every other key, host shortcuts included, goes to the nested desktop. The developer's input arrives on the nested Hyprland's own seat, not through Cua, so it never enters the Action Log.
3. Step away, from the footer button or Super+Tab, pauses the Takeover so the developer can use their own desktop, for example to fetch a secret from a password manager. keek resets the submap, sets `no_focus` again, returns focus to the developer's previous window and turns the Session Window's border yellow. The Verifier's held call stays held, the idle timeout stays paused and the recording keepalive keeps running. keek sends one desktop notification, "Takeover paused · myapp", which says the Verifier waits until Done.

   While paused, a panel over the dimmed Session Window says to click the session to carry on. No host key or click reaches the session: `no_focus` is set and the panel takes the click.
4. Resume, from the footer button or a click on the card body: keek dismisses the notification and does what Take control does. It clears `no_focus`, focuses the Session Window and enters the `keek` submap again. The Takeover carries on; its start and the screen keek compares at Done stay those of Take control.
5. Done, from the footer button or Super+Esc, works while the Takeover runs and while it is paused, and is the only way to hand control back. During a running Takeover keek resets the submap, sets `no_focus` again and returns focus to the developer's previous window. From a pause keek dismisses the notification and leaves the developer's focus where it is. Either way it then releases the held call. Instead of the call's normal result, the Verifier receives a note ("The developer pressed Done and handed control back after 0:42 (reason). If you raised a Help Request, it is answered. The screen may have changed.") and a fresh screenshot, so it re-reads the screen before acting. The note names the screenshot's pixel size, the desktop size and the scale between them. If the image size cannot be read, it says the scale is unknown and to call `get_window_state` before acting on coordinates.

   keek cannot see the developer's input, so it compares the screen at Take control with the screen at Done ([`facts.md`](facts.md#screen-comparison-results)). If the screen differs, or either screenshot cannot be read, the note is the one above. If it looks the same, keek still releases the held call, and the note says instead: "The screen looks unchanged, so if you raised a Help Request, it may not be answered. Re-read the screen, then call request_help again or skip the Check with a comment." The Verifier is told rather than assumed answered, so it does not close the session on the same blocker. A caret blink or a clock tick alone is not a change. The footer says which of the two happened, and the Help Request is recorded as unanswered.

The developer adds keek's submap snippet to their Hyprland config once. If it is missing, Take control still works, but host shortcuts stay live and the footer says so. *(my call)*

```lua
hl.define_submap("keek", function()
  hl.bind("SUPER + Escape", hl.dsp.submap("reset"))
  hl.bind("SUPER + Tab", hl.dsp.submap("keek-away"))
end)
```

Leaving the `keek` submap means Done, so Step away has its own signal: Super+Tab switches to the `keek-away` submap, and keek reads that as Step away and resets the submap at once. keek checks for the snippet through `hyprctl binds -j`. If the snippet has no Super+Tab bind, the footer says the Step away shortcut is missing, and the Step away button still works. *(my call)*

The nested desktop types as `us` during a Takeover, whatever the developer's own layout is, because the Cua plugin requires that keymap. keek's docs say so.

Paste from your own desktop during a Takeover. At Take control and at every Resume, keek copies the plain text on the host clipboard into the session's clipboard, so a 2FA code or recovery key copied from your password manager pastes with the nested desktop's paste shortcut. An image or other non-text copy is skipped and the session's clipboard keeps what it had. On Done keek sends the session a clear for its clipboard before it releases the Verifier's held call, so `clipboard_read` doesn't return what you carried in. The clear runs inside the session a few milliseconds after it is sent, and if it can't be sent, the daemon log says so. Nothing copied inside the session reaches your clipboard. If keek can't read the host clipboard or reach the session's, the Takeover still runs and the daemon log says paste is unavailable; type the text instead.

What you paste is visible on screen like anything you type, so it shows in the Raw Recording and the Verification Video. The text itself never goes on a command line, through Cua Driver, into the Action Log or into the Session Timeline. keek copies it with the hint password managers use, so the nested Omarchy shell's clipboard history skips it ([`facts.md`](facts.md#takeover-clipboard-results)).

Recording during a Takeover:

- The Raw Recording keeps running and keeps the footage locally, paused spans included.
- The Session Timeline gets one Takeover entry from Take control to Done: start, end, reason and each span the developer stepped away. keek reads the start before it clears `no_focus` and the end after it sets `no_focus` again, so the entry spans all of the developer's input. None of that input is logged.
- It also gets one Help Request entry: reason, start, end and whether it was answered, unanswered or withdrawn. An answered or unanswered Help Request ends where its Takeover starts. It is unanswered when the developer pressed Done and the screen looked unchanged. One still open when the session ends counts as withdrawn.
- The Verification Video cuts the Help Request's wait with nothing in its place, whether it was answered or unanswered.
- Each Takeover shows as about 2 s of its own footage, blurred, with "You were in control" centred on top. The text has no box: it is navy on light footage and cream on dark footage, picked from the footage's mean brightness.
- What stays on screen after Done shows unblurred in the video, so a code left visible in a field still shows after the Takeover.
- The Verdict Report keeps the wait, the time in control and the reason, and says when the Help Request was unanswered.

### Session end

When the session closes (normally or as an Incomplete Session), the sandbox stops and the Session Window closes with it. The Session Card stays at full size: keek shows the session's last frame, dimmed, in the window's place (centred on black when the window was smaller than its slot), with one panel over it. The panel is no wider than the Session Window. It shows "Rendering" and its percentage above a progress bar while keek builds the Verification Video, then the Verdict with the Verification Checklist. Each Check shows its outcome, Checks the Verifier added are tagged, and a failed or skipped Check shows its comment beneath it. A Needs fixes card also shows the attempt ("attempt 1 of 3"), counted from the Project's `index.md` for the same branch. The footer keeps the video length and gains buttons to open the Verification Video and the Evidence folder. The card stays until the developer dismisses it, which empties its place. In the rail an ended session's card shows its Verdict, and clicking it brings this full end card to focus. A new session that finds no empty place replaces the ended card that ended first; the Verdict stays in the Evidence folder. *(my call)*

## Left to the builder

- The Session Footer's layout, styling and copy follow [`DESIGN.md`](DESIGN.md).
