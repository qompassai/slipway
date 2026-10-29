# Changelog — slipway

## 1.0.0 — 2026-10-28

The 1.0 release line for the estate MCP fleet (fleet-to-1.0
program, G-W1).

- Version line: `slipway-mcp` 0.1.0 → 1.0.0. No tool-surface
  change: the same 15 tools in class order (read —
  `slipway_status`, `slipway_toolchain`, `slipway_inspect`,
  `slipway_verify`, `slipway_release_check`, `slipway_certify`,
  `slipway_bundle_check`; execute — `slipway_build`,
  `slipway_install`, `slipway_device_open`,
  `slipway_device_verify`, `slipway_device_close`,
  `slipway_drive_app`; consequential — `slipway_sign`,
  `slipway_release`), same schemas, same receipt / certification
  / bundle formats, rmcp 3.8.0, modern protocol era only
  (2026-07-28).
- Re-gated on both estate machines at 1.0.0: cargo test 18/18
  (16 unit + 2 wire) on each, `cargo fmt --check` clean,
  `cargo clippy --all-targets -- -D warnings` clean (resolved
  clippy 0.1.100, f7575a9da8, via the pinned
  nightly-2026-09-25 toolchain on both machines).
- `UTILIZATION.md` added: a real scripted modern-era stdio
  session against the 1.0.0 binary running the release-prep
  flow on a fixture artifact — certify → release bundle →
  bundle_check — including both tamper legs (a one-byte-tampered
  artifact is refused by `slipway_release` with
  `certification_mismatch`; a tampered bundle copy fails
  `slipway_bundle_check`).
- Docs currency: README gained a 1.0.0 status section;
  `docs/SERVERS-entry.md` updated to the landed 1.0.0 state
  (it previously described the pre-landing 0.1.0 build tree).
  No COMPLIANCE.md exists for this server; none was invented.
- Registry `server.json` updated to 1.0.0: the package
  identifier names the v1.0.0 MCPB and `fileSha256` records
  the 1.0.0 MCPB built on primo from the landed tree's release
  binary and smoke-tested (discover + 15 tools):
  `b61642b8a7a555833a8d30046a1ea85025a9d6d21f002217a3cdcb7c91ae768c`.
  The GitHub release upload of that MCPB is a separate act.
- Deferred, explicitly: G-W2 (witness-bundle L2 consumer check
  with fathom's witness bundle, light-show Play AAB vs F-Droid
  APK) waits on fathom's witness-bundle disposition — it is a
  consumer item, not a 1.0.0 blocker. The four `not_certified`
  certification verdicts (light-show + ONTrack × google-play +
  fdroid) remain bound to Owner-side items only; no app or
  server defect blocks them.

## 0.1.0 — 2026-10-27

- First build, from birth: rmcp 3.8.0, modern-only (no legacy
  era has ever existed in this server); 15 tools with
  annotations from birth; era guard refusing `initialize` at
  every version with the typed `-32022` error.
- Stage gates at birth (BUILD-REPORT.md): S0 pin-down refresh
  PASS (light-show AAB byte-identical to the 2026-10-24
  pin-down); S1 read core PASS; S2 build + install PASS;
  D-121 certification batteries ISSUED for light-show +
  ONTrack × google-play + fdroid (all four `not_certified`
  solely on Owner-side items; every measurable gate passes);
  D-121 modern-era client QA PASS (light-show 5/5, ONTrack
  6/6, sandbox and primo); S3 manifest-gated signing battery
  PASS 5/5 against a labelled throwaway key; S4 release-prep
  PASS (bundle assembled and re-verified; one-byte-tampered
  artifact refused with `certification_mismatch`).
- Standing gates at birth: cargo test 18/18, clippy
  `-D warnings` clean, fmt clean, Diver gate 15/15 files
  diagnostics 0 (primo mirror, live config).
- Published: crates.io `slipway-mcp` 0.1.0; registry
  `ai.qompass/slipway` 0.1.0.
