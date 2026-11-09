-- keek's overrides on top of the developer's own read-only config.

-- These autostart entries act on the machine, not on the desktop in the window.
local host_only = {
  "omarchy-provision-first-run",
  "omarchy-powerprofiles-init",
  "omarchy-hyprland-monitor-watch",
  "udiskie",
  "herdr server",
  "omarchy-hook post-boot",
}

local exec_cmd = hl.exec_cmd
hl.exec_cmd = function(command, ...)
  for _, name in ipairs(host_only) do
    if type(command) == "string" and command:find(name, 1, true) then
      return
    end
  end
  return exec_cmd(command, ...)
end

dofile(os.getenv("HOME") .. "/.config/hypr/hyprland.lua")

-- One app fills the Session Window, so Evidence shows only the app.
hl.config({ general = { gaps_in = 0, gaps_out = 0, border_size = 0 }, decoration = { rounding = 0 } })
-- The developer's window opacity would show their wallpaper through the app.
hl.window_rule({ match = { class = ".*" }, opacity = "1.0 override 1.0 override" })

hl.monitor({ output = "", mode = "preferred", position = "auto", scale = 1 })
-- Xwayland would set DISPLAY for every app, and the sandbox promises none.
hl.config({ xwayland = { enabled = false } })
hl.plugin.load("/usr/lib/cua/hyprland/cua-hyprland-plugin.so")
-- Otherwise a warning that start-hyprland wasn't used shows in every recording.
hl.config({ misc = { disable_watchdog_warning = true } })
-- The plugin opens its input socket only when this is set at load, and refuses
-- typing on any keymap but this one.
hl.config({
  plugin = { cua = { enabled = true } },
  input = {
    kb_rules = "evdev",
    kb_model = "pc105",
    kb_layout = "us",
    kb_variant = "",
    kb_options = "",
    numlock_by_default = false,
  },
})
