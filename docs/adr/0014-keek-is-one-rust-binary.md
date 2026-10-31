# keek is one Rust binary

keek needs an MCP server with progress notifications, gtk4-layer-shell Session Footers, Hyprland event-socket handling, sandbox process control and ffmpeg-driven editing, all in a long-running host process that costs no noticeable CPU while nobody looks at it. It is written in Rust as a single `keek` binary, with no runtime to install. Rust is also Cua Driver's language, so keek's Fast-forward editor can inform the upstream change for [#6211](https://github.com/trycua/cua/issues/6211).

## Considered Options

- Python with PyGObject: the fastest way to build the footer and script ffmpeg, but it needs a runtime and packaging, and is weaker as a long-running host process.
- TypeScript: a strong MCP SDK, but thinner layer-shell support, and keek would span two runtimes.
