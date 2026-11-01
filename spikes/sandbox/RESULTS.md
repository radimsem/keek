# Sandbox spike results (#2)

Run on 2026-10-31 on Omarchy with Hyprland 0.56.2, aquamarine 0.15.1, cua-hyprland-plugin 0.33.2-3, Cua Driver 0.38.0, bubblewrap 0.13.1 and pasta 2026_10_02. Session folders are in `~/.cache/keek/sessions/`.

**Status: decided on 2026-11-01.** The spread on the ticket's measure is over 200 ms and the sandbox needed changes the docs didn't describe. keek judges alignment against the page event, with a 116 ms constant and a 150 ms guard, and accepts the limits below. `docs/` holds the results. The Takeover with a person hasn't been run, and #8 and #9 now carry it.

## Acceptance criteria

| Criterion | State | Evidence |
| --- | --- | --- |
| Nested Hyprland in the sandbox through the security-context connection | Met, except the Worktree row of the Isolation table, which the ticket's plan doesn't build | Steps 1 to 3 |
| Pinned driver from its explicit path, telemetry off, sign-in recorded through the plugin's seat | Met | Sign-in below |
| No host `DISPLAY`, separate buses, no screencopy or virtual input against the host | Met | Steps 1 and 2 |
| Window floats at a fixed size with `no_focus`, and a Takeover works through the `keek` submap | Partly: float, size and silent opening are measured. `no_focus` input blocking and the Takeover aren't run | Step 4 |
| Alignment constant and spread measured and written down | Measured here, not in `docs/` | Step 5 |
| A Takeover cut leaves no Takeover footage | Met with a scripted Takeover. Not run with a person | Step 6 |

## Verification

### Step 1: environment, buses, namespaces

Inside, from a process started by the nested Hyprland, final code:

```
## env | grep -E "^(DISPLAY|DBUS_SESSION)"
DBUS_SESSION_BUS_ADDRESS=unix:path=/tmp/dbus-63ega5u0mo,guid=0a519c3fd6f071f254f46ac26ae659e3
## env | grep -E "^(WAYLAND|HYPRLAND|XDG_RUNTIME)"
HYPRLAND_CMD=Hyprland --config /keek/nested/hyprland.lua
WAYLAND_DISPLAY=wayland-1
HYPRLAND_INSTANCE_SIGNATURE=efb50993780079460b0cbed1363e2166a2de1d9f_1793481187_954046556
XDG_RUNTIME_DIR=/run/user/1000
## busctl --user list (names with an owner)
org.freedesktop.DBus 3 dbus-daemon
## readlink /proc/self/ns/net
net:[4026533310]
## readlink /proc/self/ns/pid
pid:[4026533371]
## runtime dir
dbus-1
hypr
wayland-1
wayland-1.lock
## host sockets reachable?
ls: cannot access '/run/dbus/system_bus_socket': No such file or directory
ls: cannot access '/tmp/.X11-unix': No such file or directory
ls: cannot access '/run/user/1000/bus': No such file or directory
## hyprctl instances
instance efb50993780079460b0cbed1363e2166a2de1d9f_1793481187_954046556:
	wl socket: wayland-1
## write to HOME
ok: wrote /home/radimsem/keek-probe
## write to ~/.config/hypr
touch: cannot touch '/home/radimsem/.config/hypr/keek-probe': Read-only file system
exit=1
## write to ~/.config/omarchy
touch: cannot touch '/home/radimsem/.config/omarchy/keek-probe': Read-only file system
exit=1
## write to /usr
touch: cannot touch '/usr/keek-probe': Read-only file system
exit=1
## write to session folder
ok
## outbound internet
200
## developer home hidden?
.cache
.config
.cua-driver
keek-probe
.local
radimsem
```

Host values: `DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/1000/bus`, `DISPLAY=:0`, `net:[4026531833]`, `pid:[4026531836]`.

### Step 2: globals through a security-context connection

`wayland-utils` isn't installed, so `helper globals --security-context` lists them. The four named globals: 0 of 4 exposed through the context, 4 of 4 on a plain host connection.

Exposed:

```
ext_background_effect_manager_v1 1
ext_idle_notifier_v1 2
hyprland_surface_manager_v1 2
org_kde_kwin_server_decoration_manager 1
wl_compositor 6
wl_data_device_manager 3
wl_drm 2
wl_output 4
wl_seat 9
wl_shm 2
wl_subcompositor 1
wp_alpha_modifier_v1 1
wp_color_manager_v1 3
wp_commit_timing_manager_v1 1
wp_cursor_shape_manager_v1 2
wp_fifo_manager_v1 1
wp_fractional_scale_manager_v1 1
wp_linux_drm_syncobj_manager_v1 1
wp_presentation 2
wp_single_pixel_buffer_manager_v1 1
wp_tearing_control_manager_v1 1
wp_viewporter 1
xdg_activation_v1 1
xdg_system_bell_v1 1
xdg_toplevel_tag_manager_v1 1
xdg_wm_base 7
xdg_wm_dialog_v1 1
zwp_idle_inhibit_manager_v1 1
zwp_keyboard_shortcuts_inhibit_manager_v1 1
zwp_linux_dmabuf_v1 5
zwp_pointer_constraints_v1 1
zwp_pointer_gestures_v1 3
zwp_primary_selection_device_manager_v1 1
zwp_relative_pointer_manager_v1 1
zwp_tablet_manager_v2 1
zwp_text_input_manager_v1 1
zwp_text_input_manager_v3 1
zxdg_decoration_manager_v1 2
zxdg_exporter_v2 1
zxdg_importer_v2 1
```

Hidden compared with a plain host connection:

```
ext_data_control_manager_v1
ext_foreign_toplevel_image_capture_source_manager_v1
ext_foreign_toplevel_list_v1
ext_image_copy_capture_manager_v1
ext_output_image_capture_source_manager_v1
ext_session_lock_manager_v1
ext_workspace_manager_v1
hyprland_ctm_control_manager_v1
hyprland_focus_grab_manager_v1
hyprland_global_shortcuts_manager_v1
hyprland_input_capture_manager_v1
hyprland_lock_notifier_v1
hyprland_toplevel_export_manager_v1
hyprland_toplevel_mapping_manager_v1
vicinae_hotkey_manager_v1
wp_content_type_manager_v1
wp_drm_lease_device_v1
wp_pointer_warp_v1
wp_security_context_manager_v1
xwayland_shell_v1
zwlr_data_control_manager_v1
zwlr_foreign_toplevel_manager_v1
zwlr_gamma_control_manager_v1
zwlr_layer_shell_v1
zwlr_output_manager_v1
zwlr_output_power_manager_v1
zwlr_screencopy_manager_v1
zwlr_virtual_pointer_manager_v1
zwp_input_method_manager_v2
zwp_virtual_keyboard_manager_v1
zxdg_output_manager_v1
```

### Step 3: writes

See the probe above. HOME and the session folder accept writes. `~/.config/hypr`, `~/.config/omarchy` and `/usr` fail with `Read-only file system`. `host-window.sh compare` reported `host state unchanged`, including the sha256 of `~/.config/hypr`.

### Step 4: Takeover

**Not run. It needs a person at the desktop.** Switching the host into the submap takes over the developer's keyboard, so it doesn't run unattended. To run it:

```
spikes/sandbox/host-window.sh install
spikes/sandbox/takeover_cut.py person ~/.cache/keek/sessions/takeover-1
spikes/sandbox/takeover_cut.py cut ~/.cache/keek/sessions/takeover-1 116 70
spikes/sandbox/host-window.sh remove
```

The guided flow itself has never been run.

### Step 5: alignment

32 clicks across 4 sessions on the final code, window hidden on workspace `keek`. Times are milliseconds from just before `start_recording` was forwarded.

```
session | n | click start | click end | page event | flip in video | flip-start | flip-end | flip-page
--- | --- | --- | --- | --- | --- | --- | --- | ---
align-4 | 1 | 3534.1 | 3800.9 | 3664.0 | 3573.0 | 39.0 | -227.8 | -91.0
align-4 | 2 | 4526.7 | 4798.0 | 4658.0 | 4564.1 | 37.4 | -233.9 | -93.9
align-4 | 3 | 5618.3 | 5914.2 | 5782.0 | 5686.4 | 68.1 | -227.8 | -95.6
align-4 | 4 | 6872.6 | 7273.0 | 7108.0 | 7008.7 | 136.1 | -264.3 | -99.3
align-4 | 5 | 8567.5 | 8823.3 | 8689.0 | 8595.3 | 27.8 | -228.0 | -93.8
align-4 | 6 | 9758.2 | 10014.8 | 9880.0 | 9784.1 | 25.9 | -230.6 | -95.9
align-4 | 7 | 10978.1 | 11268.1 | 11137.0 | 11039.3 | 61.2 | -228.8 | -97.7
align-4 | 8 | 12515.6 | 12785.8 | 12652.0 | 12557.9 | 42.4 | -227.9 | -94.1
align-5 | 1 | 3533.6 | 3796.4 | 3661.4 | 3565.9 | 32.3 | -230.5 | -95.5
align-5 | 2 | 4868.1 | 5118.4 | 4981.4 | 4886.9 | 18.9 | -231.5 | -94.4
align-5 | 3 | 6237.6 | 6508.7 | 6373.4 | 6274.2 | 36.6 | -234.4 | -99.1
align-5 | 4 | 7236.7 | 7500.8 | 7362.4 | 7264.7 | 28.0 | -236.2 | -97.7
align-5 | 5 | 8396.1 | 8693.7 | 8553.4 | 8453.5 | 57.4 | -240.1 | -99.8
align-5 | 6 | 10190.5 | 10504.4 | 10360.4 | 10244.3 | 53.8 | -260.0 | -116.1
align-5 | 7 | 11286.3 | 11744.2 | 11591.4 | 11498.1 | 211.8 | -246.1 | -93.3
align-5 | 8 | 13151.9 | 13450.8 | 13319.4 | 13215.4 | 63.5 | -235.4 | -104.0
align-6 | 1 | 3534.3 | 3798.7 | 3661.7 | 3566.3 | 32.1 | -232.4 | -95.4
align-6 | 2 | 5213.0 | 5667.6 | 5480.7 | 5416.6 | 203.6 | -250.9 | -64.1
align-6 | 3 | 6992.3 | 7299.8 | 7174.7 | 7069.7 | 77.3 | -230.1 | -105.0
align-6 | 4 | 8343.5 | 8713.5 | 8488.7 | 8390.8 | 47.3 | -322.8 | -98.0
align-6 | 5 | 10044.9 | 10338.2 | 10204.7 | 10108.8 | 63.9 | -229.4 | -95.9
align-6 | 6 | 11376.6 | 11668.9 | 11526.7 | 11429.2 | 52.5 | -239.8 | -97.6
align-6 | 7 | 13004.8 | 13310.5 | 13175.7 | 13080.8 | 76.0 | -229.7 | -94.9
align-6 | 8 | 14463.5 | 14916.2 | 14660.7 | 14599.8 | 136.4 | -316.4 | -60.9
align-7 | 1 | 3553.0 | 3850.7 | 3716.5 | 3632.3 | 79.3 | -218.4 | -84.1
align-7 | 2 | 4638.5 | 4910.6 | 4777.5 | 4689.0 | 50.5 | -221.6 | -88.5
align-7 | 3 | 6400.9 | 6696.1 | 6559.5 | 6472.4 | 71.5 | -223.7 | -87.1
align-7 | 4 | 7542.4 | 7816.7 | 7686.5 | 7595.1 | 52.7 | -221.6 | -91.4
align-7 | 5 | 9188.7 | 9580.5 | 9385.5 | 9314.2 | 125.5 | -266.3 | -71.3
align-7 | 6 | 10303.9 | 10708.1 | 10534.5 | 10441.2 | 137.4 | -266.9 | -93.2
align-7 | 7 | 11430.3 | 11988.2 | 11814.5 | 11763.3 | 333.0 | -224.9 | -51.1
align-7 | 8 | 13301.6 | 13923.4 | 13624.5 | 13547.4 | 245.8 | -376.0 | -77.1

clicks: 32 across 4 sessions
flip - click start: min     18.9  median     59.3  max    333.0  spread   314.1
flip - click end:   min   -376.0  median   -232.0  max   -218.4  spread   157.7
flip - page event:  min   -116.1  median    -94.7  max    -51.1  spread    64.9
C lower bound (-min of flip - page event): 116.1
align-4 flip - start: min     25.9  median     40.7  max    136.1  spread   110.2
align-5 flip - start: min     18.9  median     45.2  max    211.8  spread   193.0
align-6 flip - start: min     32.1  median     70.0  max    203.6  spread   171.6
align-7 flip - start: min     50.5  median    102.4  max    333.0  spread   282.5
```

The commands: `measure.py run SESSION 8`, then `measure.py analyze`, which uses `ffmpeg -vf "select='gt(scene,0.3)',showinfo,metadata=print:key=lavfi.scene_score"`.

An earlier run of 24 clicks, before the review fixes below, gave a flip minus click start spread of 353 ms and a flip minus page event spread of 66.6 ms. Those fixes changed the timing. Which one did it wasn't isolated.

Reading the numbers:

- Flip minus click start spreads 314 ms, over the ticket's 200 ms limit. Flip minus click end spreads 157.7 ms.
- Most of that spread is where the input lands inside the call, which takes about 250 to 620 ms because the driver captures state around each Action. It isn't the clock moving.
- Flip minus page event compares the video with a timestamp the page reads when the click lands. It spreads 64.9 ms, about one frame at the hidden window's 15 fps.
- A flip can't reach the video before the page acts, so the video's clock sits at least 116.1 ms behind the proxy's. The design says about 50 ms.

### Step 6: Takeover cut

A scripted Takeover read the clock, then turned the page magenta, and read the clock again while the page was still magenta. `takeover_cut.py cut` scans with `signalstats`. All three final runs removed about 3 s of a 10.44 s video:

| C (ms) | Guard (ms) | Magenta frames left, of 45 or 46 (cut-4, cut-5, cut-6) |
| --- | --- | --- |
| 100 | 0 | 0, 1, 1 |
| 110 | 0 | 0, 0, 0 (first magenta frame 63.0, 7.2 and 5.9 ms inside the cut) |
| 110 | 70 | 0, 0, 0 (133.0, 77.2 and 75.9 ms inside) |
| 1536, the offset of Cua's turn clock | 0 | 21, 21, 21 |

The last row shows the detector can fail, and that a cut from Cua's own times would leave footage.

The chosen values, rerun on the same three recordings after the decision. The first magenta frame was at least 161.9 ms inside the cut, and the last at least 185.5 ms inside it:

```
cut-4: magenta in raw 4491.7..7397.2 ms (45 frames); cut 4272.6..7603.3 ms with C=116 guard=150; removed 3.38 s of 10.44 s; magenta frames left: 0
cut-5: magenta in raw 4433.5..7405.4 ms (46 frames); cut 4270.4..7601.2 ms with C=116 guard=150; removed 3.28 s of 10.44 s; magenta frames left: 0
cut-6: magenta in raw 4431.6..7404.9 ms (46 frames); cut 4269.7..7590.4 ms with C=116 guard=150; removed 3.28 s of 10.44 s; magenta frames left: 0
```

C=116 with guard 0 and with guard 70 also left 0, 0, 0.

### Step 7: killing the parent

After `kill -9` on the proxy during a recording, `ps -ef | grep -E 'hyprland|wf-recorder|cua-driver'` showed only the host's own session processes, and the session window and workspace `keek` were gone. This ran before the review fixes, which didn't change the process chain.

## Sign-in

A fresh Verifier subagent with only the goal and Checks passed both Checks from screenshots, with 5 Actions and no failed tool calls. The page logged `submit ok:true`, every turn used `linux_hyprland_foreground_input`, and the video decoded (99.6 s). That run was before the review fixes. On the final code, a scripted replay of the same Actions logged `submit ok:true` with 5 turns and a decodable video.

## What the sandbox needed beyond the docs

Accepted on 2026-11-01. They change what a session can do, so the docs now state them:

- Xwayland is off, because the ticket requires `DISPLAY` absent, so an X11-only App Under Test can't run. The `DISPLAY` Hyprland set was its own Xwayland, never the host's.
- The plugin needs the `evdev/pc105/us` keymap, so a developer on another layout types `us` during a Takeover.
- Chromium refuses background input, so `click` and `type_text` need `delivery_mode:"foreground"`. keek makes that the default when it forwards a tool, so a Verifier doesn't have to pass it.
- A hidden Session Window records at the host's `misc:render_unfocused_fps` (15 here), through a `render_unfocused` window rule. A visible window wasn't measured.

Implementation facts for #3:

- pasta closes inherited descriptors above stderr, so the helper runs inside pasta: `pasta -- helper connect-exec -- bwrap`.
- pasta's defaults forward ports both ways and map the gateway to host loopback. A host listener on `127.0.0.1` was reachable from inside until `--no-map-gw -t none -u none -T none -U none`.
- `WAYLAND_SOCKET` is set for Hyprland alone. Set for the whole sandbox, it reached every bus-activated service. `dbus-run-session` still holds a copy of the descriptor, so a crashed Hyprland's window stays until the session ends.
- Bus-activated services inherit stdout, and a stray line corrupted the MCP stream, so that stream moves to private descriptors (`nested/init.sh`).
- The plugin opens its input socket only with `plugin:cua:enabled` set in the config at load. Setting it at runtime doesn't start it.
- `ATSPI_DBUS_IMPLEMENTATION=dbus-daemon` is needed, because the default `dbus-broker` fails without systemd.
- On NVIDIA, Hyprland aborts without `/dev/nvidia*` in the sandbox. `/etc` is also mounted read-only, with a resolver file pasta can serve.
- `misc.disable_watchdog_warning` keeps Hyprland's warning out of the video.
- A session window's client pid on the host is the helper's, because the helper made the connection. That answers the matching question `docs/viewer.md` leaves open.
- Driver variables, from `health_report` with each one removed: `CUA_DRIVER_RS_ENABLE_WAYLAND`, `WAYLAND_DISPLAY` and `XDG_RUNTIME_DIR` turn it degraded. Removing `HYPRLAND_INSTANCE_SIGNATURE` or `DBUS_SESSION_BUS_ADDRESS` still reports ok, but `health_report` doesn't test input.
- `xdg-desktop-portal-hyprland` used 95% of a core inside one early session. It wasn't reproduced or explained.
