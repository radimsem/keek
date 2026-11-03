# An Agent Session is a nested Hyprland in a sandbox under the developer's uid

The coding agent already runs as the developer with a shell, so the boundary around an Agent Session keeps the agent's desktop from disturbing the developer's (focus, input, clipboard, notifications, config writes, ports). It is not a wall against a hostile agent. A VM ([ADR 0001](0001-agent-session-is-a-local-vm.md)) cost a boot, several GB per session and a Guest Image that drifts from the developer's desktop. The first spike ran a nested Hyprland as the developer and found that apps inside saw `DISPLAY=:0` and the developer's D-Bus and accessibility buses. So each Agent Session runs a nested Hyprland from the host's own packages inside a bubblewrap sandbox under the developer's uid. The sandbox has:

- a throwaway HOME, with the developer's Hyprland and Omarchy config mounted read-only and keek's overrides on top (scale 1, the Cua plugin and the `us` keymap it requires, Xwayland off, no idle lock)
- its own D-Bus and accessibility bus, and no `DISPLAY`
- the worktree behind a throwaway write layer, over the Project Layer
- a private network namespace through pasta, with outbound internet and rules blocking private, shared-address, link-local and unique-local ranges during an Agent Session

The nested Hyprland reaches the host compositor through a single connection, passed in as `WAYLAND_SOCKET` and created through a `wp_security_context_v1` socket. Hyprland hides screencopy and virtual input from such connections.

## Considered Options

- Local VM (ADR 0001): the strongest separation, but slow to open, heavy, and not the developer's desktop.
- Separate Linux user: the kernel separates files, but creating the user needs root, Omarchy's factory reset removes the account, and that user needs access to the developer's Wayland socket.
- Nested Hyprland as the developer, with only the environment fixed: the App Under Test would still write into the developer's `~/.config` and caches.

## Consequences

- No root and no extra account. A session opens in seconds.
- The agent tests on the developer's installed Hyprland, apps and Omarchy config, so host updates change what it runs on. Omarchy's `cua-hyprland-plugin` package requires the exact Hyprland and aquamarine versions, and `open_session` refuses when the plugin fails to load into the nested Hyprland.
- Cua Driver and the recorder run inside the sandbox, so they see its accessibility bus and nested display.
- The App Under Test has its own localhost, so it never competes with the developer's dev servers for ports. It reaches host loopback services through ports forwarded per Project; the range limits, DNS exception and install exception are below.
- With Xwayland off, an X11-only App Under Test can't run. Hyprland's Xwayland would be the session's own and never the host's, so one config line can turn it on if a Project needs it, after checking that Cua's input and recording work with it.
- The daemon watches host windows whose client pid equals connect-exec's pid. Only the placed Session Window with class `aquamarine` is allowed. Another window, including a second `aquamarine` output, or a changed Session Window class ends the Agent Session by killing pasta and files an Incomplete Session. Checks run on `openwindow` and every second for class changes. This tripwire leaves a gap of an event, a client-list round trip and a kill, or up to one polling tick for a class change, plus the round trip and kill. It does not prevent a hostile compositor from briefly disturbing the desktop. Listing failures are logged and retried; an unknown client pid leaves the guard off.
- Before an Agent Session's sandbox starts, connect-exec installs policy rules making `10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, `100.64.0.0/10`, `169.254.0.0/16`, `fc00::/7` and `fe80::/10` unreachable at priority 20. The DNS forwarder at `169.254.1.1` uses the main table at priority 10. Rules precede the main table even for connected routes; the local table still precedes the rules. Any rule failure refuses the Agent Session, including an IPv6 failure. Sandboxed code cannot remove these rules.
- A Project needing a LAN service uses a host loopback tunnel and a forwarded port. Install keeps LAN access for private package mirrors. Public-addressed LAN neighbours and other special ranges outside the list remain reachable. The DNS exception covers all ports at the forwarder address.
