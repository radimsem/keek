# Cua Driver performs Actions and records the session

Cua Driver writes the video, a 30 Hz cursor track and a record of each Action inside the Agent Session, so every Action passes through one tool that also films it. Omarchy's own `omarchy-capture-screenrecording` is built for a person at the desk (region picker, shell indicator, one recording per user runtime dir). So keek records through Cua Driver and does not use Omarchy's screen recorder. This ADR first assumed Cua's turn times shared the video's clock. They run about 1.45 s behind it on 0.38.0, so keek times the session itself ([ADR 0013](0013-keek-times-the-session-on-stock-cua-driver.md)).

## Consequences

- keek pins the driver version, stock 0.38.0 for now; the plugin's package pins the Hyprland version.
- Cua lists Hyprland support as experimental. The spikes proved browser input and recording in a nested Hyprland, first on its own and then inside the sandbox.
- Cua's action records store full arguments, including typed text. Masking typed secrets belongs in Cua Driver ([ADR 0009](0009-shared-pieces-go-to-cua-driver.md)). Until a release carries it, keek scrubs every Test Credential value when it files Evidence.
