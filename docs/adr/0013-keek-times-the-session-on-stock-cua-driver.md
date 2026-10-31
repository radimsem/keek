# keek times the session itself, on stock Cua Driver

keek's implementation shouldn't wait on upstream reviews, and Cua Driver 0.38.0's turn times read about 1.45 s behind its video ([#6210](https://github.com/trycua/cua/issues/6210)). That gap isn't cosmetic: a Takeover cut made from those times would end early and leave the last 1.45 s of the Takeover, possibly a 2FA code, in the Verification Video. Every Cua call already passes through keek's MCP proxy, so keek runs stock Cua Driver and keeps its own Session Timeline: it reads a monotonic clock just before it forwards `start_recording` and times every call from there. Cua starts wf-recorder right after its own anchor (`recording.rs`, `video_wayland.rs`), so this zero sits about 50 ms before the video's first frame on both 0.38.0 and a build with the fix. keek corrects for that with a small measured constant.

Until a Cua release carries them, keek also carries two stopgaps: an ffmpeg editor for the Verification Video in place of Fast-forward in `recording render`, and a Test Credential scrub in place of masking in `action.json`.

## Considered Options

- Run a Cua Driver fork with the fix from [PR #6239](https://github.com/trycua/cua/pull/6239): no timing code in keek, but keek builds and pins a fork, and Fast-forward and masking would block on fork work.
- Add 1.5 s to Cua's turn times: simple, but stock and fixed recordings use the same fields, so keek would have to remove the correction at exactly the right driver version or correct twice.

## Consequences

- The Session Timeline stays after #6239 ships, because Check marks and Takeovers need a timeline anyway. Only the two stopgaps get removed, each through a `stopgap` ticket blocked on its upstream release.
- Replacing the editor with Cua's render needs both Fast-forward and the #6239 fix in a release, because that render reads Cua's own turn times.
- On 0.38.0, the times in filed `actions/` disagree with the video. keek leaves them as recorded, and the Verdict Report names the Session Timeline as its time source.
- The alignment is reasoned from source until the next spike measures it.
