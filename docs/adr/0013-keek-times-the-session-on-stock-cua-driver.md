# keek times the session itself, on stock Cua Driver

keek's implementation shouldn't wait on upstream reviews, and Cua Driver 0.38.0's turn times read about 1.45 s behind its video ([#6210](https://github.com/trycua/cua/issues/6210)). That gap isn't cosmetic: a Takeover cut made from those times would end early and leave the last 1.45 s of the Takeover, possibly a 2FA code, in the Verification Video. Every Cua call already passes through keek's MCP proxy, so keek runs stock Cua Driver and keeps its own Session Timeline: it reads a monotonic clock just before it forwards `start_recording` and times every call from there. Cua starts wf-recorder right after its own anchor (`recording.rs`, `video_wayland.rs`), so this zero sits a short, steady time from the video's clock. The sandbox spike measured it on 0.38.0 at 116 ms, and keek subtracts that constant. A Takeover cut reaches a further 150 ms past each side.

The spike's flip minus click start spread 314 ms, over the 200 ms limit it set for itself. keek judges alignment against the page's own timestamp instead, where the spread is 65 ms. A click call takes 250 to 620 ms and its effect lands at varying points inside it, so the larger spread describes the call, not the clock. Takeover cuts are what a wrong clock would break, and they use keek's own times for the Takeover's edges, not a call's start. The editor keeps each whole call plus margins, and every measured effect fell inside its call.

Until a Cua release carries them, keek also carries three stopgaps: an ffmpeg editor for the Verification Video in place of Fast-forward in `recording render`, a Test Credential scrub in place of masking in `action.json`, and recovery hints for pid and capture refusals in place of Cua's own.

## Considered Options

- Run a Cua Driver fork with the fix from [PR #6239](https://github.com/trycua/cua/pull/6239): no timing code in keek, but keek builds and pins a fork, and Fast-forward and masking would block on fork work.
- Add 1.5 s to Cua's turn times: simple, but stock and fixed recordings use the same fields, so keek would have to remove the correction at exactly the right driver version or correct twice.

## Consequences

- The Session Timeline stays after #6239 ships, because Check marks and Takeovers need a timeline anyway. Only the three stopgaps get removed, each through a `stopgap` ticket blocked on its upstream release.
- Replacing the editor with Cua's render needs both Fast-forward and the #6239 fix in a release, because that render reads Cua's own turn times.
- On 0.38.0, the times in filed `actions/` disagree with the video. keek leaves them as recorded, and the Verdict Report names the Session Timeline as its time source.
- The constant holds for one machine and 0.38.0, measured with a hidden window. A driver upgrade reruns the measurement, and a build with the #6239 fix hasn't been measured.
- A call's start on the Session Timeline isn't the moment its effect shows, so anything that needs that moment has to look at the video.
