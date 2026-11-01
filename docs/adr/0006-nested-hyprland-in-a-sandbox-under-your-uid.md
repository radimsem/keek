# An Agent Session is a nested Hyprland in a sandbox under the developer's uid

The coding agent already runs as the developer with a shell, so the boundary around an Agent Session keeps the agent's desktop from disturbing the developer's (focus, input, clipboard, notifications, config writes, ports). It is not a wall against a hostile agent. A VM ([ADR 0001](0001-agent-session-is-a-local-vm.md)) cost a boot, several GB per session and a Guest Image that drifts from the developer's desktop. The first spike ran a nested Hyprland as the developer and found that apps inside saw `DISPLAY=:0` and the developer's D-Bus and accessibility buses. So each Agent Session runs a nested Hyprland from the host's own packages inside a bubblewrap sandbox under the developer's uid. The sandbox has:

- a throwaway HOME, with the developer's Hyprland and Omarchy config mounted read-only and keek's overrides on top (scale 1, the Cua plugin and the `us` keymap it requires, Xwayland off, no idle lock)
- its own D-Bus and accessibility bus, and no `DISPLAY`
- the worktree behind a throwaway write layer, over the Project Layer
- a private network namespace through pasta, with outbound internet

The nested Hyprland reaches the host compositor through a single connection, passed in as `WAYLAND_SOCKET` and created through a `wp_security_context_v1` socket. Hyprland hides screencopy and virtual input from such connections.

## Considered Options

- Local VM (ADR 0001): the strongest separation, but slow to open, heavy, and not the developer's desktop.
- Separate Linux user: the kernel separates files, but creating the user needs root, Omarchy's factory reset removes the account, and that user needs access to the developer's Wayland socket.
- Nested Hyprland as the developer, with only the environment fixed: the App Under Test would still write into the developer's `~/.config` and caches.

## Consequences

- No root and no extra account. A session opens in seconds.
- The agent tests on the developer's installed Hyprland, apps and Omarchy config, so host updates change what it runs on. Omarchy's `cua-hyprland-plugin` package requires the exact Hyprland and aquamarine versions, and `open_session` refuses when the plugin fails to load into the nested Hyprland.
- Cua Driver and the recorder run inside the sandbox, so they see its accessibility bus and nested display.
- The App Under Test has its own localhost, so it never competes with the developer's dev servers for ports. It reaches host services only through ports forwarded per Project.
- With Xwayland off, an X11-only App Under Test can't run. Hyprland's Xwayland would be the session's own and never the host's, so one config line can turn it on if a Project needs it, after checking that Cua's input and recording work with it.
