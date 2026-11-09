# keek is an Omarchy layer on Cua Driver, and shared pieces go upstream first

Checked on 2026-10-30, Cua has no session that runs on the developer's own Omarchy. Its Spaces are containers (XFCE on Xvfb) or VMs, including a stock Omarchy edge VM for amd64. Its desktop app supports only Ubuntu 24.04. Cua Cloud ended on 18 October 2026, and `trycua/e2e-test-agent`, which produces a verdict, report and recording for a PR, runs in GitHub CI against a Cua desktop pool. keek's own value is the sandboxed nested session on the developer's Omarchy, the live view with Takeover, and Evidence filed by Project. Three pieces keek needs would serve every Cua Driver user, so they are proposed to Cua Driver and belong there in the end:

- Hyprland video aligned with the Action Log. Cua Driver already records the nested Hyprland through wf-recorder, but the spike's video ran 1.45 s behind the Action Log.
- Idle Fast-forward in `cua-driver recording render`, timed from the Action Log and labelled with real time ([ADR 0004](0004-fast-forward-idle-instead-of-cutting.md)).
- Masking typed secrets in `action.json`, which stores them as plain text today.

Each starts as an issue on `trycua/cua`. keek doesn't wait for them and runs no fork: it runs stock Cua Driver with its own stopgaps until a Cua release carries each piece ([ADR 0013](0013-keek-times-the-session-on-stock-cua-driver.md)). keek keeps the sandboxed nested session, the viewer, Takeovers (including their blurred, compressed footage in the Verification Video), its MCP tools and Evidence filing.

## Considered Options

- All in Cua: propose the nested host session as a new Space kind and build the viewer into Cua. Cua's runtimes are containers, VMs and microVMs, and Spaces is source-available (FSL-1.1-MIT), not MIT.
- Standalone keek: build recording alignment, Fast-forward and masking in keek. Faster to control, but the work would not reach other Cua users and keek would carry it alone.

## Consequences

- A piece Cua declines stays in keek.
- Issue and pull request text is approved by the developer before it is posted.
- Cua's render knows nothing of Takeovers. keek splits a session at each Takeover, edits each part, and joins the parts with about 2 s of each Takeover's own footage, blurred, with "You were in control" centred on top. It keeps doing so once Cua's render does the editing.
- Chromium on Hyprland stays outside the apps Cua has tested its Hyprland plugin with (LibreOffice Calc and Inkscape). The spike is keek's only evidence that it works.
