# keek viewer design

How the developer watches Agent Sessions and takes them over on their Omarchy desktop. Terms are in [`GLOSSARY.md`](GLOSSARY.md); the session, recording and Evidence design is in [`design.md`](design.md).

## Intent

The developer codes while one or two agents verify changes in the background. Every so often they glance at the agents to answer one question: is each one making progress, or stuck? When an agent hits something it cannot get past (a 2FA prompt, a captcha, an OS dialog), the developer takes over that one session briefly, then hands it back and the agent carries on.

The model is the watch, take control, done pattern of Grok Bot, Manus and Anthropic's computer-use demo. Because each Agent Session is a nested Hyprland, its desktop is already a live window on the developer's screen, and keek only adds what that window lacks ([ADR 0007](adr/0007-nested-window-is-the-session-window.md)).

## Goals

- A glance tells the developer, per session: which Project and session it is, what it is verifying, what it did last and how long ago, and whether it needs them.
- A session that needs the developer gets their attention even when they are not looking at the agents.
- The developer can drive any session on demand, the agent waits meanwhile, and both sides know exactly when control changed hands.

## Non-goals

- Watching more than a couple of sessions at once. keek caps open sessions at two (configurable); the layout does not need to scale to a fleet.
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
- The workspace has one slot per allowed open session: two by default, each half the monitor's width. A Session Card fills its slot. Without `window` in the Project Profile, the Session Window fills the slot above the Session Footer, so the Verification Video size follows the developer's monitor. The Project Profile can set a different size, such as a phone size for a mobile web app. A smaller Session Window is centred on black at its true size. The agent's desktop and the Verification Video keep the Session Window size.
- A Session Window opens there silently: it never takes focus or switches the developer's workspace. It closes when its session ends. Its window rule includes `render_unfocused`, so the session keeps rendering and recording while the workspace is hidden.
- keek ships no keybinding to reach the workspace. Its docs suggest one for the user's Hyprland config. *(my call)*

### Viewer process and narrow monitors

GTK runs on the daemon process main thread; the Tokio daemon runs on a worker thread. A bounded channel carries Session Footer lifecycle messages, and each footer awaits its snapshot watch channel on GLib. If GTK cannot initialise, the daemon continues without a viewer. Hyprland workspace events control visibility, and the elapsed timer exists only while at least one footer is visible.

Two slots sit side by side on monitors at least 1920 logical pixels wide (960 pixels per half before gaps). Narrower monitors stack the two cards vertically, splitting the usable height. The Session Window keeps its opening size throughout the session.

### Session Window

The nested Hyprland's own window. It shows the agent's desktop live, with Cua's agent cursor, its action marks and a ripple on each click in the picture.

keek sets `no_focus` on it from the moment it maps until a Take control. The developer's clicks, scrolls and keys never reach the agent's desktop by accident, and the window can't be dragged or resized out of its slot.

keek's host rule matches class alone, so a nested Hyprland the developer starts themselves while a session is open is caught by it too: it opens floated on the `keek` workspace with `no_focus`, and keek lifts neither while a session is open. keek places each Session Window by its client pid, so the developer's window normally stays where the rule left it; only when the pid cannot be resolved does keek take the first new `aquamarine` window, which can be the developer's. Moving `no_focus` and the placement to per-window props once the pid matches would leave a mapped window focusable and off `keek` for a moment, and the Takeover boundary needs `no_focus` from map time (ADR 0007), so the class rule stays.

When the last session closes, disabling the rule lifts `no_focus` but not the placement. keek then moves each `aquamarine` window still on `keek` to the active workspace and tiles it, without switching to it. It moves only a window whose client process is named `Hyprland`: a Session Window belongs to a keek process, so another daemon's Session Windows stay where they are. While `keek` itself is the active workspace the window stays floating there, since a tiled window would cover the Session Cards. A daemon that stops on SIGTERM with a session open also leaves it on `keek`, because keek moves windows only when a last session closes. A window left either way is moved the next time a last session closes while another workspace is active.

### Session Footer

A gtk4-layer-shell surface per slot, shown only while the `keek` workspace is active. keek follows Hyprland's workspace events to show and hide it.

Once the session ends, the footer moves to the bottom layer with the rest of the card, so a window the developer opens on `keek` covers the whole Session Card.

It sits flush under its Session Window and overlaps the window's bottom border by the Hyprland rounding (8 px), so window and footer read as one Session Card. The footer always has its slot's width and sits at the slot's bottom. Around a Session Window smaller than its slot, the window floats centred above the footer instead of resting on it, and the black backdrop draws the card's outer edge in the window's current border colour and joins the footer's side borders; the window keeps its own border as a frame. The footer draws its own side and bottom border in the window's current border colour and rounds only its bottom corners. The nested desktop is sized so the overlap hides nothing but the border. A 2 px top edge in the session's cursor colour ties the card to its Cua cursor, on screen and in the video. *(my call)*

It shows that session's:

- Project and branch (`myapp`, `fix/17-login-redirect`), after a small arrow in the session's cursor colour. The start time stays in the Evidence file names, not on screen.
- Goal.
- Last Action in plain words (`click "Sign in"`) and the time since it, counting up.
- The Verification Checklist's progress ("Checked 2 of 4", counting marked Checks) with one tick per Check: filled green for passed, filled red for failed, an outlined yellow dash for skipped, a dim outline for not yet marked.
- Elapsed time since the session opened, so a stuck session stands out. Once the session ends it reads "Ran" with the total.
- Help Request state: the whole card turns orange, the window border through a window rule and the footer and any black backdrop with it, and the footer shows the Verifier's reason.
- During a Takeover: "You are in control" and its running time, with the window border in the active colour.
- The **Take control** button, which becomes **Done** during a Takeover.

The keek daemon already sees every Action on its way to Cua Driver, so it pushes an update to the footer per Action and per state change; nothing polls. *(my call)*

### View-only hint

Outside a Takeover, `no_focus` makes Hyprland skip the Session Window in pointer hit tests, so the pointer lands on whatever is beneath it. keek puts a layer-shell surface on the bottom layer under each slot to catch that hover. It covers the whole card body, including the black sides around a Session Window smaller than its slot, which stay inert during a Takeover. While the pointer is over the card body, the footer's state column reads "View only" and "Use Take control to interact.", and the Take control button lights up. A click there does the same and nothing else. The footer shows the last Action again when the pointer leaves. *(my call)*

### Help Request

- The Verifier calls `request_help(session, reason)`. The call blocks until a Takeover on that session ends, sending progress notifications while it waits.
- That session's Session Card turns orange (the Omarchy needs-you colour) and its footer shows the reason. keek sends one critical desktop notification ("Agent needs you: 2FA code · myapp") through `omarchy-notification-send`, so it stays on screen until the developer acts, and dismisses it when the developer presses Take control. The shell dismisses by summary, so this also clears a second waiting session's notification; that session's card stays orange. Activating the notification switches to the `keek` workspace. *(my call)*
- If the agent cancels the `request_help` call before the developer presses Take control (Esc in the coding agent, or a client timeout such as Codex's 5 minutes of active time), keek withdraws the Help Request: the card and footer return to their earlier state, the orange border clears and the Verifier's next call runs at once. The notification stays. A Takeover already running when the cancel arrives continues until Done. Only `request_help` is withdrawn, not a forwarded Cua call or `close_session`.
- There is no timeout while a Help Request waits for the developer. The 5-minute idle timeout (see [`design.md`](design.md#abnormal-end)) pauses during a Help Request and a Takeover.

### Takeover

A Takeover starts only from the **Take control** button. Looking at, hovering over or clicking the Session Window never starts one.

1. Take control: keek holds the Verifier's next tool call (any forwarded Cua tool, or the pending `request_help`). Holding comes first, so no Verifier Action can land after the developer starts.
2. keek clears `no_focus`, focuses the Session Window and switches the host Hyprland into the `keek` submap. That submap binds only Super+Esc, which means Done, so every other key, host shortcuts included, goes to the nested desktop. The developer's input arrives on the nested Hyprland's own seat, not through Cua, so it never enters the Action Log.
3. Done, from the footer button or Super+Esc: keek resets the submap, sets `no_focus` again, returns focus to the developer's previous window and releases the held call. Instead of the call's normal result, the Verifier receives a note ("The developer pressed Done and handed control back after 0:42 (reason). If you raised a Help Request, it is answered. The screen may have changed.") and a fresh screenshot, so it re-reads the screen before acting. The note names the screenshot's pixel size, the desktop size and the scale between them. If the image size cannot be read, it says the scale is unknown and to call `get_window_state` before acting on coordinates.

The developer adds keek's submap snippet to their Hyprland config once. If it is missing, Take control still works, but host shortcuts stay live and the footer says so. *(my call)*

```lua
hl.define_submap("keek", function()
  hl.bind("SUPER + Escape", hl.dsp.submap("reset"))
end)
```

keek checks for the snippet through `hyprctl binds -j`.

The nested desktop types as `us` during a Takeover, whatever the developer's own layout is, because the Cua plugin requires that keymap. keek's docs say so.

The nested desktop doesn't bind the host clipboard, so type a 2FA code or password during a Takeover instead of pasting it ([`facts.md`](facts.md#takeover-clipboard-results)).

Recording during a Takeover:

- The Raw Recording keeps running and keeps the footage locally.
- The Session Timeline gets one Takeover entry: start, end and reason. keek reads the start before it clears `no_focus` and the end after it sets `no_focus` again, so the entry spans all of the developer's input. None of that input is logged.
- It also gets one Help Request entry: reason, start, end and whether it was answered or withdrawn. An answered Help Request ends where its Takeover starts. One still open when the session ends counts as withdrawn.
- The Verification Video cuts the Help Request's wait with nothing in its place.
- Each Takeover shows as about 2 s of its own footage, blurred, with "You were in control" centred on top. The text has no box: it is navy on light footage and cream on dark footage, picked from the footage's mean brightness.
- What stays on screen after Done shows unblurred in the video, so a code left visible in a field still shows after the Takeover.
- The Verdict Report keeps the wait, the time in control and the reason.

### Session end

When the session closes (normally or as an Incomplete Session), the sandbox stops and the Session Window closes with it. The Session Card stays at full size: keek shows the session's last frame, dimmed, in the window's place (centred on black when the window was smaller than its slot), with one panel over it. The panel is no wider than the Session Window. It shows "Rendering" and its percentage above a progress bar while keek builds the Verification Video, then the Verdict with the Verification Checklist. Each Check shows its outcome, Checks the Verifier added are tagged, and a failed or skipped Check shows its comment beneath it. A Needs fixes card also shows the attempt ("attempt 1 of 3"), counted from the Project's `index.md` for the same branch. The footer keeps the video length and gains buttons to open the Verification Video and the Evidence folder. The card stays until the developer dismisses it, which empties the slot. A new session that needs the slot replaces the card; the Verdict stays in the Evidence folder. *(my call)*

## Left to the builder

- The Session Footer's layout, styling and copy follow [`DESIGN.md`](DESIGN.md).
