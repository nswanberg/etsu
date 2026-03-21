# Changelog

## 2026-03-15

### Added
- **UTC is now explicit, with optional machine-local capture context.** ETSU continues to store `metrics.timestamp` as the canonical UTC instant and can also populate `timestamp_local` plus `local_utc_offset_minutes`. Supabase REST sync only sends those extra fields when the remote schema already exposes them, so the rollout can be staged safely behind a downstream migration.

### Fixed
- **ETSU now blocks startup until macOS permissions are granted.** Previously it would start the input listener regardless, silently capturing nothing if Input Monitoring or Accessibility were missing. Now it polls every 2 seconds and only proceeds once both are confirmed.
- **`setup_macos.sh` auto-restarts ETSU after permissions are granted.** macOS requires a process restart for Input Monitoring grants to take effect. The script now detects the waiting state, periodically restarts the LaunchAgent, and confirms input capture is working before reporting success.
- **macOS signing helper now trusts the generated cert, and app installs sign by identity hash.** The original helper imported the keypair but did not make the self-signed cert trusted for `codeSign`, so `security find-identity` could still report zero valid identities on some Macs. `install_app_bundle.sh` also now signs with the resolved identity hash instead of the display name, which avoids `codesign` ambiguity if duplicate `ETSU Development` labels exist in the keychain.
- **`create_macos_dev_signing_identity.sh` SIGPIPE fix.** `tr -dc ... | head -c 32` triggers SIGPIPE under `set -o pipefail`. Added `|| true` to the pipeline.
- **`create_macos_dev_signing_identity.sh` OpenSSL 3.x compatibility.** macOS `security import` rejects PKCS12 files created by OpenSSL 3.x without legacy encoding. Added `-legacy` flag to `openssl pkcs12 -export`.

### Changed
- **`setup_macos.sh` permission flow.** The script now prints clear instructions when permissions are missing, auto-opens the remaining privacy pane, waits up to 90 seconds by default for the user to grant them, auto-restarts the agent to pick up grants, and exits nonzero if capture is still blocked. The timeout is configurable via `ETSU_PERMISSION_WAIT_TIMEOUT_SECONDS`.
