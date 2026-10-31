---
status: superseded by ADR-0006
---

# An Agent Session is a local VM, not a nested Hyprland

An Agent Session must keep input and screen, desktop services, files and config, and system state away from the host. A second headless or nested Hyprland under the same user only separates input and screen: it still shares the user's shell IPC, D-Bus session, notification daemon, `~/.config`, systemd user units and packages. A separate Linux user adds file isolation but not system state. So each Agent Session is a QEMU/KVM VM on the developer's machine, and the project reaches it through a read-only share with a throwaway write layer.

## Considered Options

- Nested or headless Hyprland under the same user: cheapest and fastest to open, but the agent's shell, toasts and config edits reach the host session.
- Separate Linux user running a headless Hyprland: isolates files, still shares packages, units and the network.
- Container: Cua's own guidance says Hyprland needs a GPU or KMS device that containers do not have.

## Consequences

- Each Agent Session costs several GB of RAM and a VM boot, so the number of open sessions is capped (one per worktree, two by default).
- The guest has no GPU encoder, so recording encodes on the CPU (wf-recorder).
- Only x86_64 hosts are supported while Cua's Omarchy image is amd64 only.
- The VM gets outbound internet and nothing else: the host and the local network are blocked.
