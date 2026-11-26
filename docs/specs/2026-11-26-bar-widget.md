# Bar Widget: keek in the Omarchy bar, and a keek icon on its notifications

Design agreed with the developer on 2026-11-26. Terms follow [`GLOSSARY.md`](../GLOSSARY.md); two new ones are proposed at the end. The screens are drawn on the bar widget canvas (https://claude.ai/artifact/N8B6hNpFGfJBN6V8F4MehZ), which reuses the palette, wallpaper and cursor glyph of the viewer proposals canvas.

## Why

The developer reaches the Agents Workspace with a Super+Shift+K bind, but nothing in the Omarchy bar shows it. Hyprland gives a named workspace a negative id, and both Omarchy's stock workspaces widget and the third-party `tornikegomareli.spaces` widget skip every workspace with `id <= 0` (`Spaces.qml:208`). So while sessions run, the bar says nothing about them, and there is nothing to click to get to `keek` and back.

keek's notifications also carry no icon. keek calls `omarchy-notification-send` without `--app-name` or `-i`, so its toasts show the default sender and can't be told apart from other apps at a glance.

## Goals

- While at least one session is on the Agents Workspace, the bar shows a keek button with one dot per session.
- Clicking the button switches to `keek`. Clicking it while on `keek` goes back to the workspace the developer came from.
- The button turns orange while a Help Request waits, and a click then shows that session.
- Hovering the button lists each session as its rail card does: Project, branch, elapsed time, state line, Checks and its Super F key.
- keek's notifications show the keek logo in the toast's icon slot.

## Non-goals

- A Waybar module. Omarchy 4 runs the Quickshell shell, and that is the only bar this targets.
- Controlling a session from the bar: no Take control, Done or close.
- Moving keek to a numbered workspace.
- Showing `keek-rail`. It stays a workspace nobody views.

## Anti-goals

- The bar ever naming a shortcut that does nothing. The Super F keys and Super+Shift+K show only when `hyprctl binds -j` lists them, the same rule the rail cards follow ([`viewer.md`](../viewer.md)).
- The widget stealing focus or switching workspace on its own, for example when a Help Request arrives.
- keek editing the developer's `shell.json` or bar layout.
- The widget polling. It reacts to Hyprland events and to file changes, following Omarchy's rule for background work.

## Constraints

- keek is standalone, not Omarchy core ([ADR 0002](../adr/0002-standalone-not-omarchy-core.md)), and keek ships no keybinding ([`viewer.md`](../viewer.md)).
- `omarchy plugin add <git-url>` clones a repository and reads `manifest.json` at its root (`/usr/share/omarchy/bin/omarchy-plugin-add`). A plugin in a subfolder of the keek repository can't be added that way.
- A bar widget is a QML plugin with `"kinds": ["bar-widget"]`. It builds on the shell's `BarWidget` and `WidgetButton` and its `Style` tokens, as `radimsem.workspaces` does. Those give the height, vertical centring, spacing and theme colours. The widget draws only its state backgrounds, the session dots and the orange fill.
- The toast picks its icon in this order: the sender's image, an icon resolved from the theme or a desktop entry for the app, an Omarchy glyph, then the app name's first letter (`plugins/notifications/Toast.qml`). The slot is 40 px.
- Session state lives in the daemon. The widget can see Hyprland's workspaces but not Help Requests, Checks or Verdicts.

## Approaches

| Approach | Trade-off | Outcome |
| --- | --- | --- |
| A keek widget plugin that keek publishes state for | Shows session state and never shows `keek-rail`. Needs a small daemon change and a separate repository. | **Chosen** |
| Patch the developer's spaces widget to show named workspaces except `keek-rail` | No keek change, but no session state, and an update of that widget undoes it. | Rejected |
| Move keek to a numbered workspace | Collides with the developer's workspaces and Omarchy's Super+number binds. | Rejected |

The developer chose session state over a Hyprland-only toggle, and distribution through Omarchy's plugin catalog rather than a folder the developer links by hand.

## Design

### Daemon: the bar state file

The daemon writes `bar.json` in its runtime folder (`$XDG_RUNTIME_DIR/keek/bar.json`, beside `daemon.sock`). It writes the file whenever a session opens, changes state, ends or is closed, and removes it when no session is left on the Agents Workspace. Each write goes to a temporary file in the same folder and is renamed over the old one, so the widget never reads a half-written file.

The daemon also removes the file on SIGTERM and when it starts. After a daemon crash the file can stay and the bar shows sessions that are gone, until the next daemon starts. That is accepted as rare. A liveness signal isn't worth its complexity. *(my call)*

The file holds a schema `version` and one entry per session on the Agents Workspace, ordered by `n`. A widget that meets a version it doesn't know hides itself. *(my call)*

- `n`: the session's number, the one its Super F key and `keek-show <n>` use. A new session can reuse a freed number (`src/registry_rail.rs`), so `n` order isn't always opening order.
- `project`, `branch`, `color` (the session's cursor colour), `opened_at`.
- `state`: working, needs you, Takeover, or ended.
- `detail`: the rail card's state line (the last Action, the Help Request reason, or the Verdict).
- `checks`: one mark per Check, passed, failed, skipped or unmarked.
- `in_focus`: whether it is in the focus slot.

It holds no Test Credential, no Action arguments beyond the state line the rail card already shows, and nothing from inside the Session Window.

### Widget

The widget is a separate repository, `radimsem/omarchy-keek`, MIT-licensed like keek, with `manifest.json` at its root. Its plugin id is `radimsem.keek`. The id is written into users' `shell.json`, so it never changes. The manifest sets `"defaultSection": "left"`. The widget releases on its own, and `bar.json`'s `version` is the contract between the two. The developer creates the repository, and the builder prepares its contents.

- **Visibility.** The widget reads `bar.json` with Quickshell's `FileView` and watches it for changes. With no file, or no sessions in it, the widget has zero width.
- **Button.** One `WidgetButton` holding the keek cursor glyph, then one 7 px dot per session in its cursor colour. Its states match the canvas:
  - sessions open: the occupied-pill background, outline glyph
  - on keek: the active-pill background, filled glyph
  - needs you: orange `#fba35e` fill, dark text, "Needs you" label, and a white ring on the waiting session's dot

  An ended session's dot shows as a ring in its colour *(my call; not drawn)*.
- **Takeover.** While a Takeover runs or is paused, the button shows the in-control state and does nothing on click, and the list's rows are dimmed and inert. This matches the rail and notifications, which don't swap during a Takeover ([`viewer.md`](../viewer.md)). Super+Tab and Super+Esc stay the way out. The daemon therefore publishes the Takeover phase in `bar.json`.
- **Click.** Off `keek`, a click switches to `name:keek`. If a session needs you, the click also emits `keek-show <n>` for it, as activating the notification does. On `keek`, a click goes back to the previous workspace, with the same rule as the spaces widget's click-active-to-go-back: the last workspace focused before `keek`, never `keek-rail`. The widget records that workspace from `Hyprland.focusedWorkspace`, since `workspacev2` events miss a move to a workspace on another monitor, and Hyprland's own "previous" can name `keek-rail` after a rail swap.
- **Hover list.** A popup under the button with one row per session, laid out like a rail card. Clicking a row switches to `keek` and emits `keek-show <n>`. The header shows Super+Shift+K, and each row shows its Super F key, only when that bind exists.
- **Accessibility.** The button's label names the session count and the needs-you state, as on the canvas.

The widget dispatches through `hyprctl dispatch` with the Lua dispatchers the stock widgets use (`hl.dsp.focus`, `hl.dsp.event`).

### Notification icon

keek keeps sending as Omarchy's default app, `omarchy-action`, and adds `-i keek`. *(my call)* The shell lets only `omarchy-action` through Do Not Disturb and never stores its toasts in history (`plugins/notifications/Delivery.js`). With `--app-name keek`, a Help Request would be silenced under Do Not Disturb, and an old toast could come back after a shell restart with its `--exec` pointing at an ended session. The toast tries the sent icon name first (`Icons.js`), so `-i keek` alone should show the logo. That is untested: if the live check shows no logo, this comes back to the developer before switching the app name. The daemon installs the logo as theme icons at start, under `$XDG_DATA_HOME/icons/hicolor/<size>/apps/keek.png` for 48, 64 and 128 px. It writes a file only when it is missing or its bytes differ, so a keek upgrade refreshes the icon. The PNGs are downscaled from `assets/logo.png` (512 px) and kept in the repository beside it, so the shell never scales the 512 px logo down to 40 px at runtime. An SVG replaces them if a vector source of the logo is made.

## Distribution

- The widget repository is added with `omarchy plugin add <url> --enable`, and keek's README says so.
- The developer wants it listed in Omarchy's public plugin catalog. How a plugin gets into that catalog was not found in the local Omarchy tools. Nothing is submitted until the developer approves the exact listing text.

## Verification

- Unit: the daemon writes, updates and removes `bar.json` as sessions open, need help, end and close; a reader never sees a partial file. Passes `now` in, so no clock is needed, and uses a temp runtime folder.
- Unit: the notification call carries `-i keek` and no `--app-name`, and the icon install writes each size once and rewrites only changed bytes.
- Live (`live_*`, `#[ignore]`): open two sessions, raise a Help Request, and check that `bar.json` and the notification call match. The developer's workspace, focus and windows end as they began.
- Manual checklist *(my call)*: the widget installed and enabled is a precondition, never something a test sets up, since keek must not edit `shell.json`. A person checks the button's states, the toggle back to the previous workspace and the toast icon against the canvas. Quickshell gives a test no way to read a widget's state.

## Left to the builder

- The exact `bar.json` field names and how `checks` is encoded.
- QML structure inside the widget repository, and its own tests if the catalog requires any.
- The popup's dismiss behaviour.

## Proposed glossary terms

- **Bar Widget**: the Omarchy bar button that shows open sessions and switches to the Agents Workspace and back.
- **Bar State**: the file the daemon publishes for the Bar Widget.
