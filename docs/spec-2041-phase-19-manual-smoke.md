# SPEC-2041 Phase 19 — manual smoke checklist

T-138 (Gate 3) requires a human reviewer to drive a real `gwt` GUI
build against an actual GitHub Release because the post-click modal
flow couples WebView rendering, WebSocket events, helper subprocess
spawn, and OS process lifecycle in ways the Playwright fixture cannot
exercise.

SPEC-2041 FR-066 codifies this gate: **CI green is not done. Phase 19
PRs may not be marked closed until this checklist passes on a release
candidate.**

Record the outcome in the Board with JSON operation `board.post`
and reference SPEC-2041
Phase 19 (FR-066, SC-014d).

## Prerequisites

1. macOS arm64 (primary target). Linux x86_64 / arm64 and
   Windows x86_64 are best-effort — same procedure, different
   download asset name.
2. Two real GitHub Releases:
   - `vN` (current): the build the reviewer will install first.
   - `vN+1` (target): the build the modal will download. Either
     publish a real next release, or use an unreleased internal tag
     against the mock server if `vN+1` is not yet ready.
3. A clean `~/.gwt/pending-update/` directory before each run so the
   bootstrap path starts from a known state:
   ```sh
   rm -rf ~/.gwt/pending-update
   ```
4. Optional — if testing against the local mock server in lieu of a
   real release. Pass **bare semver** to `--version` (the script prepends
   `v` itself, so `--version v1.2.3` would emit `vv1.2.3` and the updater's
   `parse_tag_version` would reject the tag):
   ```sh
   node scripts/mock-update-server.cjs --port 18080 --version 9.99.0 \
     --asset path/to/real/gwt-macos-arm64.tar.gz
   GWT_UPDATE_API_BASE_URL=http://127.0.0.1:18080 ./target/release/gwt
   ```

## Gate 3.A — Restart-now happy path

1. Install `vN` from the release asset matching your platform.
2. Launch `gwt`. Wait up to 5 minutes (or restart with the polling
   override) until the bottom-right `Update available: v<N+1> — Click
   to update` CTA appears.
3. Click the CTA. **Verify**: a centered modal opens immediately with
   the title "Updating gwt", a progress bar, and a `Cancel` button.
   The CTA itself flips to "Applying update..." and is disabled.
4. **Verify**: the progress bar advances and the byte counter
   (`X.Y MB / Z.W MB`) updates roughly every 200 ms while the
   download proceeds. Update progress events should arrive over
   WebSocket without UI freezes.
5. When the download completes the modal transitions to "Update
   ready" with `[Later]` and `[Restart now]` buttons. **Verify**:
   no `window.confirm` appears.
6. Click `[Restart now]`. **Verify**: the gwt window closes within
   ~3 seconds and a new gwt window opens. Open the About / version
   line and confirm it now reports `vN+1`.

## Gate 3.B — Later → next-launch transparent apply

1. Re-install `vN` (or rerun the smoke from the Gate 3.A starting
   state if you can roll back to the manifest-cleared state).
2. Launch `gwt`, click the CTA, wait for the `ready` modal.
3. Click `[Later]`. **Verify**:
   - the modal closes,
   - the CTA morphs to `Update v<N+1> ready — Restart now` with a
     visible dismiss `×`,
   - `~/.gwt/pending-update/manifest.json` exists and points at the
     prepared payload under `~/.gwt/updates/v<N+1>/...`.
4. Quit gwt manually (cmd-Q / window close).
5. Launch `gwt` again. **Verify**:
   - the bootstrap path detects the manifest, swaps the binary, and
     starts a new process running `vN+1`. The transition should be
     close to instant — the user sees a momentary launch flash and
     then the regular gwt window for the new version.
   - `~/.gwt/pending-update/` no longer contains a manifest.

## Gate 3.C — Failure UX (expected)

1. Re-install `vN`.
2. Launch the mock server with no `--asset` so it returns 32 bytes of
   garbage. Pass `--version` as bare semver (e.g. `9.99.0`) — the
   script prepends `v` itself:
   ```sh
   node scripts/mock-update-server.cjs --port 18080 --version 9.99.0
   GWT_UPDATE_API_BASE_URL=http://127.0.0.1:18080 ./target/release/gwt
   ```
3. Click the CTA. **Verify**: progress reaches 100 % (32 / 32 bytes)
   then the modal transitions to the `failed` state with:
   - title `⚠ Update failed`,
   - `Stage:` "Persist pending" or "Download asset" depending on
     where extract_archive aborts,
   - `Reason:` something like "Failed to prepare update payload: …",
   - `Log:` a path under `~/.gwt/logs/update-<YYYY-MM-DD>.log`,
   - three buttons: `[Open log] [Retry] [Close]`.
4. Click `[Open log]`. **Verify**: the OS default text viewer opens
   the log file and the file contains JSONL entries for
   `download_start`, `download_complete`, `fail` with the same
   reason as the modal.
5. Click `[Close]`. **Verify**: the modal closes and the CTA returns
   to `Update available: v<N+1> — Click to update`.

## Gate 3.D — Cancel mid-download

1. Re-install `vN`. If you control the mock server, throttle it
   (e.g. wrap with `pv` to slow the body) so the download takes
   noticeably more than ~1 s.
2. Click the CTA, then click `[Cancel]` while the progress bar is
   below 100 %. **Verify**:
   - the modal closes,
   - the CTA returns to `Update available`,
   - polling resumes (no manifest is created),
   - any partial file under `~/.gwt/updates/v<N+1>/` is harmless
     (the next start_download will overwrite it).

## Reporting

Post a single Board entry summarising the outcome:

```bash
gwtd <<'JSON'
{"schema_version":1,"operation":"board.post","params":{"kind":"status","body":"SPEC-2041 Phase 19 Gate 3 manual smoke result:\n- 3.A Restart-now: PASS / FAIL\n- 3.B Later + next-launch swap: PASS / FAIL\n- 3.C Failure UX: PASS / FAIL\n- 3.D Cancel mid-download: PASS / FAIL\nNotes: <free-form observations, screenshots, log excerpts as needed>"}}
JSON
```

Message body:

```text
SPEC-2041 Phase 19 Gate 3 manual smoke result:
- 3.A Restart-now: PASS / FAIL
- 3.B Later + next-launch swap: PASS / FAIL
- 3.C Failure UX: PASS / FAIL
- 3.D Cancel mid-download: PASS / FAIL
Notes: <free-form observations, screenshots, log excerpts as needed>
```

If any sub-gate fails, do not merge dependent work and reopen the
relevant Phase 19 PR or file a follow-up issue against SPEC-2041
referencing the failing FR.
