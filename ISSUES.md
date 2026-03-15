# Known Issues

## macOS permissions silently lost after rebuild (critical, now fixed)

**Symptom:** ETSU process runs (visible in `ps`) but records zero keypresses, clicks, or scroll events. The log file stops updating. Data silently stops flowing to SQLite and Supabase with no user-visible indication.

**Root cause:** macOS ties Input Monitoring and Accessibility grants to the binary's code signature. Ad-hoc signatures (`codesign --sign -`) produce a different hash on every build, so every `cargo build --release` + reinstall silently invalidates the previous permission grant. The old process keeps running but receives no input events from `rdev`. There was no check or warning at runtime.

**What went wrong (2026-03-15):** ETSU had been running since Saturday morning but silently stopped capturing input around 3 PM on March 14. The process appeared healthy (PID present, launchd status 0) but had produced no data for ~21 hours. Approximately 21 hours of keycount data was permanently lost.

An attempt to fix the permissions by running `tccutil reset ListenEvent` and `tccutil reset Accessibility` wiped Input Monitoring and Accessibility permissions for **all apps on the system**, not just ETSU. This is because `tccutil reset` operates on the entire permission category, not per-app. Every app that had these permissions had to be re-authorized.

**Fix applied:**
1. `create_macos_dev_signing_identity.sh` creates a persistent local codesigning identity ("ETSU Development") stored in the macOS keychain. This identity survives rebuilds.
2. `install_app_bundle.sh` automatically finds and uses this identity if it exists, falling back to ad-hoc only if it doesn't.
3. `main.rs` now blocks startup until both Input Monitoring and Accessibility are confirmed, polling every 2 seconds instead of logging a warning and proceeding without input.
4. `setup_macos.sh` now auto-restarts the process after the user grants permissions (macOS requires a restart for grants to take effect) and waits for confirmation before reporting success.

**Remaining issues:**
- `create_macos_dev_signing_identity.sh` had two bugs: a SIGPIPE from `tr | head` under `set -o pipefail`, and OpenSSL 3.x producing PKCS12 files that macOS `security import` rejects without the `-legacy` flag. Both are fixed.

## `tccutil reset` is system-wide, not per-app

## macOS "quit and reopen" after granting Input Monitoring

When granting Input Monitoring to a running app, macOS shows a "quit and reopen" dialog. For a launchd-managed daemon with `KeepAlive = true`, this works out: macOS kills the process, launchd restarts it, and the new process picks up the grant. But if the setup script isn't monitoring for this, it can appear to hang or report failure. The setup script now handles this by periodically restarting the agent while waiting for permissions.
