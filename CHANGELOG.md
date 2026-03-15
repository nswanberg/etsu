# Changelog

## 2026-03-15

### Fixed
- **ETSU now blocks startup until macOS permissions are granted.** Previously it would start the input listener regardless, silently capturing nothing if Input Monitoring or Accessibility were missing. Now it polls every 2 seconds and only proceeds once both are confirmed.
- **`setup_macos.sh` auto-restarts ETSU after permissions are granted.** macOS requires a process restart for Input Monitoring grants to take effect. The script now detects the waiting state, periodically restarts the LaunchAgent, and confirms input capture is working before reporting success.
- **`create_macos_dev_signing_identity.sh` SIGPIPE fix.** `tr -dc ... | head -c 32` triggers SIGPIPE under `set -o pipefail`. Added `|| true` to the pipeline.
- **`create_macos_dev_signing_identity.sh` OpenSSL 3.x compatibility.** macOS `security import` rejects PKCS12 files created by OpenSSL 3.x without legacy encoding. Added `-legacy` flag to `openssl pkcs12 -export`.

### Changed
- **`setup_macos.sh` permission flow.** The script now prints clear instructions when permissions are missing, waits up to 120 seconds for the user to grant them, auto-restarts the agent to pick up grants, and confirms input capture before finishing. Previously it checked once and exited with a warning.
