# The Session Window is the nested Hyprland window itself

A nested Hyprland already shows the agent's desktop as a live window on the developer's desktop, so streaming it into a keek window ([ADR 0005](0005-native-gtk-session-window-on-cua-stream.md)) would add a capture path and latency for nothing. keek uses the nested window as the Session Window and sets `no_focus` on it, which blocks the developer's focus and pointer until Take control. keek floats it at a fixed size in one of two slots on the Agents Workspace. A nested Hyprland's output always takes its host window's size, and nothing pins it, so re-tiling would resize the agent's desktop and its video mid-recording. Every nested window has the same class and title (`aquamarine`, `aquamarine - WAYLAND-1`), so the Session Footer, a layer-shell bar joined under each window, identifies the session. It shows the Project, branch, goal, last Action and any Help Request, plus the Take control button. During a Takeover keek switches the host Hyprland into a `keek` submap, so every key except the one that means Done reaches the nested desktop.

## Considered Options

- GTK4 window on Cua's spacesd stream (ADR 0005): built for a VM that has no window of its own on the host.
- Hide the nested window and capture it into a keek window: a steady size on any monitor, but a capture stream, added latency and a hidden window to manage.
- No keek UI, only notifications and a CLI: least code, but nothing shows at a glance which session is which.

## Consequences

- A session's size is set when it opens: about half the Agents Workspace's width, or a size the Project sets. The Verification Video has that size.
- `no_focus` also skips pointer hit tests, so the developer can't drag or resize a Session Window by accident. That keeps it paired with its Session Footer.
- The developer adds keek's submap snippet to their Hyprland config once.
- A nested window on a hidden workspace stops rendering unless it has a `render_unfocused` window rule. With the rule, it records at the host's `misc:render_unfocused_fps` (15 by default), which keek leaves alone.
- The plugin requires the `us` keymap in the nested Hyprland, so a developer types `us` during a Takeover whatever their own layout is.
- keek enforces the Takeover boundary (`no_focus` plus held agent calls), not Cua's stream. The developer's input reaches the nested desktop through its normal seat and the agent's through the plugin's seat, so nothing the developer types enters the Action Log.
