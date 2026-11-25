---
name: keek
description: Watch a fresh Verifier check the agent's change on your own Omarchy desktop, and take control when it needs you.
colors:
  bar-navy: "#193e58"
  control-navy: "#1f4a6a"
  parchment: "#eee2cc"
  parchment-dim: "#c9c2b6"
  parchment-comment: "#ddd3c0"
  harbor-blue: "#77baf3"
  harbor-blue-text: "#bcdcff"
  border-active: "#6ba7d8"
  border-inactive: "#316184"
  ink-on-accent: "#0f2d42"
  needs-orange: "#fba35e"
  pass-green: "#a9c972"
  fail-coral: "#ff8e7c"
  skip-yellow: "#ebcf70"
  scrim: "rgba(12, 36, 54, 0.86)"
  session-cyan: "#4ccce0"
  session-violet: "#a77bf3"
  session-pink: "#f58fc2"
  session-green: "#8fd6a0"
typography:
  verdict-title:
    fontFamily: "JetBrains Mono, monospace"
    fontSize: "22px"
    fontWeight: 700
  panel-title:
    fontFamily: "JetBrains Mono, monospace"
    fontSize: "18px"
    fontWeight: 700
  check:
    fontFamily: "JetBrains Mono, monospace"
    fontSize: "14px"
    fontWeight: 400
    lineHeight: "20px"
  detail:
    fontFamily: "JetBrains Mono, monospace"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: "18px"
  body:
    fontFamily: "JetBrains Mono, monospace"
    fontSize: "12px"
    fontWeight: 400
  tag:
    fontFamily: "JetBrains Mono, monospace"
    fontSize: "11px"
    fontWeight: 400
rounded:
  sm: "4px"
  md: "8px"
spacing:
  xs: "4px"
  sm: "6px"
  md: "10px"
  lg: "12px"
  xl: "16px"
  2xl: "24px"
components:
  session-footer:
    backgroundColor: "{colors.bar-navy}"
    textColor: "{colors.parchment}"
    typography: "{typography.body}"
    rounded: "{rounded.md}"
    padding: "2px 16px 0"
    height: "76px"
  session-footer-control:
    backgroundColor: "{colors.control-navy}"
    textColor: "{colors.parchment}"
  button-take-control:
    backgroundColor: "rgba(119, 186, 243, 0.10)"
    textColor: "{colors.harbor-blue-text}"
    typography: "{typography.body}"
    rounded: "{rounded.md}"
    padding: "0 18px"
    height: "40px"
  button-take-control-hover:
    backgroundColor: "rgba(119, 186, 243, 0.22)"
  button-take-control-needs:
    backgroundColor: "{colors.needs-orange}"
    textColor: "{colors.ink-on-accent}"
    rounded: "{rounded.md}"
    padding: "0 18px"
    height: "40px"
  button-done:
    backgroundColor: "{colors.harbor-blue}"
    textColor: "{colors.ink-on-accent}"
    rounded: "{rounded.md}"
    padding: "0 22px"
    height: "40px"
  button-secondary:
    backgroundColor: "rgba(238, 226, 204, 0.04)"
    textColor: "{colors.parchment}"
    rounded: "{rounded.md}"
    padding: "0 16px"
    height: "40px"
  verdict-panel:
    backgroundColor: "{colors.bar-navy}"
    textColor: "{colors.parchment}"
    rounded: "{rounded.md}"
    padding: "28px 32px"
    width: "720px"
  help-request-toast:
    backgroundColor: "{colors.bar-navy}"
    textColor: "{colors.parchment}"
    rounded: "{rounded.md}"
    padding: "14px 16px 16px"
    width: "380px"
  added-tag:
    textColor: "{colors.parchment-dim}"
    typography: "{typography.tag}"
    rounded: "{rounded.sm}"
    padding: "1px 6px"
---

# Design System: keek

## Overview

**Creative North Star: "The Glass-Walled Workshop"**

keek is a workshop with glass walls on the developer's own desktop. The agent works in plain view, in its own room, and the developer can look in at a glance, walk in when called, and walk out again. Every surface keek adds is the frame of that room: it labels who is inside, what they did last, and what came of it. The room itself, the agent's nested Omarchy desktop, is never restyled.

The system borrows everything it can from the host. Omarchy's bar, Hyprland's borders and 8px rounding, JetBrains Mono and the theme palette carry over unchanged, so a Session Card reads as one more Omarchy window with a footer, not as an app. keek's own vocabulary is small and strict: one footer, one Verdict panel, one toast, one cursor. Density is that of a status bar, not a dashboard; a glance answers "is it progressing, is it stuck, does it need me".

Colour is reserved for meaning. Navy and parchment carry everything at rest. Orange appears only when the developer is needed, green and coral only for Check outcomes, yellow for Check outcomes and a paused Takeover, and each session's cursor colour only where it ties the card to its cursor.

The approved comps live in `design/comps/`, kept outside git on the developer's machine, and in the design canvas (https://claude.ai/artifact/LxCSK6QwgvUrNBKL8ftXzq). Values below are measured from those comps on the Cumulus theme; on any other theme the colours follow that theme (see Colors).

**Key Characteristics:**
- Native to Omarchy: host bar, borders, rounding and type, no keek chrome on the agent's desktop.
- Window and footer join into one Session Card.
- Status-bar density: label over value, one line each.
- Colour by meaning only; state also readable by shape and word.
- Flat: borders and a dim scrim make depth, never shadows.

## Colors

A dark Omarchy navy carrying parchment text, with a small set of signal colours that each mean one thing. Skip Yellow is the one exception: it marks skipped Checks and Partly verified, and also a paused Takeover. The paused card always says "Takeover paused", so word and place tell the two apart.

keek takes these roles from the active Omarchy theme. The values in the frontmatter are the Cumulus theme's, and the comps show them. Resolve each role with `omarchy-theme-color`, which applies Omarchy's own fallbacks, and the two border roles the way Omarchy's Hyprland template does.

| Role | Theme key |
| --- | --- |
| Bar Navy | `background` |
| Control Navy | `lighter_background` (Cumulus `#254b67`; the comps used `#1f4a6a`) |
| Parchment | `foreground` |
| Parchment Dim | `dark_foreground` |
| Parchment Comment | between `foreground` and `dark_foreground` |
| Harbor Blue | `accent` |
| Harbor Blue Text | `bright_blue` |
| Border Active | `hyprland_active_border`, else `accent` |
| Border Inactive | `hyprland_inactive_border`, else `rgba(595959aa)` |
| Ink on Accent | `darker_background` |
| Needs-you Orange | `orange` (falls back to `yellow`) |
| Pass Green / Fail Coral / Skip Yellow | `green` / `red` / `yellow` |
| Scrim | `darker_background` at 86% |
| Session Cyan / Violet / Pink / Green | fixed session colours, not themed |

Three themes in Omarchy's set have no `orange`, so a Help Request and a skipped Check share a hue there. Shape and wording still tell them apart.

### Primary
- **Harbor Blue** (accent): the developer's own actions and keek's progress. Done button, rendering progress bar, toast border and countdown, the "View only" label.
- **Harbor Blue Text**: text on the outlined Take control button and the "You are in control" label.

### Secondary
- **Needs-you Orange**: a Help Request and nothing else. Window border, footer border, solid Take control, toast icon tile.

### Tertiary
- **Pass Green**: passed Checks and the Pass Verdict.
- **Fail Coral**: failed Checks and the Needs fixes Verdict.
- **Skip Yellow**: skipped Checks and the Partly verified Verdict, and a paused Takeover: the card border, the "Takeover paused" label and the paused panel's border.
- **Session Cyan / Session Violet / Session Pink / Session Green**: one per open session, in the order sessions take their numbers. The Cua cursor, the footer's 2px top edge, the rail card's session edge and the arrow before the project name.

### Neutral
- **Bar Navy**: footer, Verdict panel and toast background; the Omarchy bar's own colour.
- **Control Navy**: footer background during a Takeover only.
- **Parchment**: primary text, Incomplete and rendering titles.
- **Parchment Dim**: labels ("Checked", "Elapsed"), branch, detail lines, meta.
- **Parchment Comment**: Check comments under a Check.
- **Border Inactive**: Session Card border at rest, panel border, divider under the Verdict header.
- **Border Active**: Session Card border during a Takeover; also the nested Chromium window border.
- **Ink on Accent**: text on solid Harbor Blue and Needs-you Orange buttons.
- **Scrim**: dims the session's last frame under the Verdict panel.

### Named Rules
**The Orange Means You Rule.** Needs-you Orange marks a Help Request only. Never use it for warnings, skipped Checks or Partly verified.

**The Session Colour Rule.** A session's colour appears on its cursor, its footer's top edge and its arrow glyph, and nowhere else. It never colours state, text or buttons.

**The Outcome Palette Rule.** Green, coral and yellow belong to Check outcomes. A Verdict's colour is the colour of the outcome that decided it. Yellow's one other use is a paused Takeover, which always carries the words "Takeover paused".

## Typography

**Body Font:** JetBrains Mono (with monospace)

**Character:** One monospace family at every size, matching Omarchy's bar and terminals. Hierarchy comes from size and weight steps, never from a second family.

### Hierarchy
- **Verdict title** (700, 22px): the Verdict word in its outcome colour. One per panel.
- **Panel title** (700, 18px): "Rendering the verification video" and its percentage.
- **Check** (400, 14px, 20px line): a Check's text in the Verdict checklist.
- **Detail** (400, 13px, 18px line): the panel subtitle and meta, Check comments, and the footer project name at 700.
- **Body** (400, 12px): every footer field, button labels, toast subtitle. Footer values step to 700.
- **Tag** (400, 11px): the "added" tag on a Check the Verifier added.

### Named Rules
**The Tabular Clock Rule.** Elapsed, Ran and percentages use tabular numerals so a running clock never shifts its column.

**The Label Over Value Rule.** Footer columns are a dim label line over a bold value line. Never put a label and value on one line.

## Layout

The reference layout is the Agents Workspace on a 3440×1440 output: the 26px Omarchy bar, then the focus slot at x 24, top 50, a 2822×1298 Session Window with its 76px Session Footer below at the same width, and the rail beside it. Gaps are Hyprland's 24px `gaps_out`.

- **Focus slot:** one Session Card. With one session open there is no rail, the card spans the workspace (3392 wide) and the focus-size window sits centred on black.
- **Rail:** a 560px column at x 2856, 10px (`gaps_in` twice) right of the focus slot, from top 50. About a sixth of the usable width, rounded down to 20px. Rail cards are 560×384, 11px apart, stacked from the top, and close up when one leaves.
- **Narrow monitors** (under 1920 logical pixels): the focus slot spans the width on top and the rail is a row below the Session Footer, its cards splitting the width 11px apart. The focus slot gives up height for the row.
- Every Session Window opens at the focus slot's size, wherever its session starts.

The Session Footer is a single row, 16px side padding, 24px between columns, centred vertically:

1. Identity block (grows): arrow glyph, project, branch on line one; goal on line two. Branch and goal truncate with an ellipsis.
2. State column (up to 560px, shrinks first): state label over its detail line.
3. Checked column (190px): "Checked" over "2 of 4" and its ticks. Hidden when the session has no Checks yet.
4. Elapsed column (96px): "Elapsed" while running, "Ran" once ended.
5. Actions (400px, right-aligned).

Ended sessions keep the last frame under the Scrim with one Verdict panel centred on it: 720px wide, 560px for the rendering panel. Checklists list Checks with 12px between rows.

The Help Request toast sits 24px from the right edge, at top 50, 380px wide.

## Elevation & Depth

keek is flat. Depth comes from the host's own window borders, the footer's border joining the window's, and the Scrim that pushes an ended session's last frame back behind its Verdict panel. Panels, toasts and footers carry no shadow. The one glow in the system is the Cua cursor's (`drop-shadow(0 0 4px <session colour>)`), which keeps the agent's pointer findable over any page.

### Named Rules
**The Flat Host Rule.** If Omarchy's own surfaces don't cast a shadow, keek's don't either.

## Shapes

Corners follow Hyprland's 8px rounding everywhere keek draws a container: Verdict panel, toast, buttons. The Session Footer rounds only its bottom corners, so it continues the window above it. Small inline marks (kbd keys, the "added" tag, footer ticks) use 2 to 4px. Window and footer borders are 2px; panels and the toast use 1px. The Cua cursor is the arrow path `M3 3 L24 11 Q17 17 11 24 Z` with a 2.6px white stroke, and every keek arrow glyph uses the same path.

### Named Rules
**The One Card Rule.** The footer overlaps the window's bottom 8px, draws its side and bottom border in the window's current border colour, and changes with it: inactive at rest, active during a Takeover, yellow while a Takeover is paused, orange for a Help Request.

## Components

### Session Card
- **Character:** the nested Hyprland window and its Session Footer, read as one card.
- **States:** at rest (Border Inactive), Help Request (whole card Needs-you Orange, footer background stays Bar Navy), Takeover (Border Active, footer Control Navy), paused Takeover (whole card Skip Yellow, footer Bar Navy, the Session Window dimmed under the paused panel), ended (last frame dimmed, one Verdict panel).
- **Behaviour:** a new session that needs the slot replaces an ended card.
- **Session Window:** fills the slot above the Session Footer when the Project Profile sets no `window`. A smaller window is centred on black at its true size, and no panel over it is wider than the window.

### Session Footer
- **Character:** a status bar for one session; quiet unless it needs the developer.
- **State labels:** "Last action" (Parchment Dim), "Needs you" (orange), "You are in control" (Harbor Blue Text), "Takeover paused" (Skip Yellow) over the time away ("0:41 away · your shortcuts work again"), "Back to the Verifier" (Harbor Blue Text), "Rendering" (Parchment Dim), then the Verdict in its outcome colour: "Pass", "Needs fixes", "Partly verified". An Incomplete Session has no Verdict and reads "Incomplete" (Parchment Dim).
- **Hand-back lines:** under "Back to the Verifier", "Handed back after 1:12. Help Request answered." when the developer answered a Help Request, "Handed back after 1:12." when no Help Request was behind the Takeover, and "The screen looks unchanged. The Verifier was told." when Done changed nothing.
- **Reminder:** "Reminded the Verifier to mark Checks" (Parchment Dim) over "8 actions since the last mark", until the Verifier's next call. A Help Request, a Takeover and the view-only hint take precedence.
- **View-only hint:** while the pointer is over a window the developer doesn't control, the state column reads "View only" (Harbor Blue) over "Use Take control to interact.", and Take control lights up.
- **Top edge:** 2px in the session colour across the full width.

### Checked ticks
- **Shape:** 10px squares, 2px radius, 4px apart, after "2 of 4".
- **Outcomes:** passed is a filled green square, failed a filled coral square, skipped a yellow outline with a dash, unmarked a dim parchment outline.
- **In progress:** while the Verifier works, the first unmarked Check's tick keeps its size but swaps the dim outline for a Harbor Blue one, with a 4px Harbor Blue dot centred inside, 3px from the outline on every side. Nothing is drawn over the square, so the row doesn't shift when the mark moves on. Not during a Help Request, a Takeover or after the session ends. A rail card marks the tick the same way.
- **In a Check list:** the row in progress is bold and carries a "checking" tag: 11px Harbor Blue Text in a pill with a 1px Harbor Blue outline at 65%.
- **Overflow:** at most 8 ticks, then "+n".
- **Accessible name:** "2 passed, 1 failed, 1 skipped, 0 not checked yet".

### Buttons
- **Shape:** 40px tall, 8px radius, 12px bold label.
- **Take control:** outlined Harbor Blue at 10% fill, 22% on hover or with the view-only hint. During a Help Request it becomes solid Needs-you Orange with Ink on Accent text.
- **Step away:** the secondary style, preceded by its `Super` `Tab` keys, before Done during a Takeover. Without the bind in the `keek` submap it shows without keys, and the state detail says the shortcut is missing.
- **Resume:** the outlined Harbor Blue style of Take control, before Done while a Takeover is paused.
- **Done:** solid Harbor Blue, preceded by the `Super` `Esc` keys it shares a shortcut with during a running Takeover. While paused it shows without keys, because the shortcut works only inside the `keek` submap.
- **Secondary (Open video, Open folder):** parchment outline at 40%, 4% fill.
- **Dismiss:** a 40px square, transparent, 12px cross.

### Verdict panel
- **Character:** the outcome of one session, readable without opening the video.
- **Header:** a 32px outcome icon, the Verdict title, the "Verified <branch> at <sha>" or "Checked <branch> at <sha>" line, and meta right-aligned beside it ("3 passed", "1 failed, 1 skipped · attempt 1 of 3", "2 passed, 1 skipped", "1 of 3 checked").
- **Icons:** Pass a check in a circle, Needs fixes an exclamation in a circle, Partly verified a half-filled circle, and an Incomplete Session a dashed circle with a pause.
- **Checklist:** one row per Check with a 16px glyph (tick, cross, dash, open circle), the Check's text, an "added" tag when the Verifier added it, and its comment beneath in Parchment Comment.
- **Rendering:** title and percentage on one line, a 6px Harbor Blue progress bar beneath.

### Rail card
- **Character:** one session at a glance; the whole card is a button that shows the session ("Show billing"). No other controls.
- **Frame:** 2px Border Inactive, 8px radius, Bar Navy. On hover Border Active and Control Navy. During a Help Request Needs-you Orange.
- **Still:** 556×256, the Session Window scaled to the card's inner width, on black, refreshed every 3 s. An age tag ("Still · 3 s ago") sits 8px from its bottom-left corner: 11px Parchment on Bar Navy at 88%, 4px radius. An ended card shows its last frame without a tag.
- **Session edge:** 2px in the session colour, under the still.
- **Text block:** 14px by 16px padding, three rows spread evenly at 12px. Row one: arrow glyph, project in 13px bold, branch in Parchment Dim with an ellipsis, elapsed time bold and right-aligned. Row two: the last Action in Parchment Dim, "Needs you" in bold orange followed by the reason in Parchment, or the Verdict in its outcome colour. Row three: "2 of 4" in bold, the ticks, and the `Super` `F2` keys right-aligned in Parchment Dim (sessions 1 to 4 only, and only once the developer has bound that key; without the bind the row ends at the ticks).
- **Locked:** at 55% opacity while a Takeover is running or paused, with one line under the rail: "Other sessions unlock when you press Done." (12px Parchment on Bar Navy at 88%, 8px radius, 10px by 14px padding, rail width).

### Rail card hover panel
- **Placement:** 440px wide, 12px left of the card, top edges aligned.
- **Style:** Bar Navy, 2px Border Active, 8px radius, 16px by 18px padding, 12px between blocks.
- **Content:** the goal, then one row per Check (12px square, 10px gap, 9px between rows; passed rows have a filled green square with a tick, failed coral, skipped a yellow outline, unmarked rows are Parchment Dim, the row in progress bold with its "checking" tag), then "Click to show this session." in Parchment Dim.

### Paused panel
- **Character:** the one overlay on a running session's desktop. The developer put it there by stepping away, and it goes on Resume or Done.
- **Dim:** the Session Window under `darker_background` at 55%.
- **Panel:** centred over the window and no wider than it. Bar Navy, 2px Skip Yellow border, 8px radius, 20px by 28px padding, "Takeover paused" in 15px bold Skip Yellow over "Click the session to carry on. The Verifier waits until you press Done." at 13px.
- **Behaviour:** a click on it, or anywhere on the card body, resumes the Takeover.

### Takeover paused notification
- **Content:** "Takeover paused · billing" over "The Verifier waits until you press Done. Go back to keek and click the session to carry on."
- **Behaviour:** sent once per Step away the way the Help Request toast is, and dismissed on Resume or Done.

### Help Request toast
- **Style:** Bar Navy, 1px Harbor Blue border, 28px orange-tinted icon tile with the arrow glyph, "Agent needs you" over the reason and project ("2FA code · billing"), and a 2px Harbor Blue countdown bar along the bottom.
- **Behaviour:** sent once through `omarchy-notification-send`; activating it switches to the `keek` workspace.

### Cua cursor
- **Arrow:** the session colour with a 2.6px white stroke and a 4px glow.
- **Settings keek sets:** Action marks and click ripple on, float off. Marks are 2px strokes in the session colour beside the arrow (observe, click, type, keys, scroll); the ripple is two rings at 60% and 25% opacity.
- **Badge:** a faint 28px circle (7% fill) under the arrow.
- **Human pointer:** stays the host's white arrow, so the two never read as one.

## Planned: Session Footer state column

Designed and approved, not built yet. The screens are in the proposals canvas (https://claude.ai/artifact/JzPWohPGuYshxKJPGUfit9), drawn at 1:1 on a 3440×1440 output with the parts above, and the behaviour is in [`specs/2026-11-22-viewer-enhancements.md`](specs/2026-11-22-viewer-enhancements.md). Values here are measured from that canvas. A ticket that builds one of these moves its rows into the sections above and updates the reference layout.

### Session Footer changes
- **State column:** a fixed 420px. Its left edge never moves; the label and the detail line truncate on the right.

## Do's and Don'ts

### Do:
- **Do** take the bar, borders, rounding (8px), gaps (24px) and font from the host so the Session Card reads as an Omarchy window.
- **Do** pair every outcome colour with an icon shape and a word.
- **Do** keep the footer to label-over-value columns at 12px, with values in bold.
- **Do** show "Needs fixes" with the attempt count ("attempt 1 of 3") so a retry reads as progress.
- **Do** check new work against the comps in `design/comps/` before calling it done.

### Don't:
- **Don't** restyle or overlay the agent's nested desktop while it runs; keek draws only around it. The paused panel is the one exception, and only while the developer has stepped away.
- **Don't** call a Verdict "Fail" or show a bare red failure state.
- **Don't** use Needs-you Orange for anything but a Help Request.
- **Don't** add shadows, gradients or a second font family.
- **Don't** put the session start time on screen; it lives in the Evidence file names.
- **Don't** steal focus, pop over the developer's editor or animate while nothing needs them.
