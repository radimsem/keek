---
status: superseded by ADR-0007
---

# Session Windows are native GTK4 on Cua's spacesd stream

The Session Window shows an Agent Session live and lets the developer take it over. Cua already serves an HTML5 viewer for each sandbox, and gliff already decodes a Hyprland stream into a GTK4 window, so keek could have wrapped either. keek builds a native GTK4 window instead, matching the rest of the Omarchy desktop, and has it read Cua's spacesd media stream directly. Cua's stream already separates view-only from input-enabled access and tags input from a viewer as human-origin, which an agent cannot claim, so the Takeover boundary is enforced by Cua rather than by keek alone.

## Considered Options

- Cua's HTML5 viewer in an app-mode browser window: least code, but a browser window that keek styles from the outside.
- gliff's crates with `gliff-server` in the Guest Image: a proven GTK4 decode path, but an extra guest service, no view-only mode, and keek alone would have to keep agent and human input apart.

## Consequences

- keek implements a client for Cua's media protocol (an `OpenMedia` ticket, then H.264 over WebSocket or QUIC), which Cua ships as alpha. Protocol changes in Cua break the Session Window until keek follows.
- The guest encodes the live stream in software, on top of the recording, so the Session Window streams only while the Agents Workspace is visible.
