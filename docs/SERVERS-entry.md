# Census entry for SERVERS.md

> Prepared by the slipway from-birth build (2026-10-27) for the
> fleet census at `~/workspace/mcp-conformance-20261019/SERVERS.md`;
> updated at the 1.0.0 release (2026-10-28) to the landed state.
> Landing the entry into SERVERS.md itself is a separate act;
> this file is the entry text.

## slipway

- **What:** the estate's Android build / sign / verify / install /
  release MCP server. Builds the estate's own apps with their
  pinned invocations, verifies artifacts by measurement (manifest
  read-back, per-.so 16 KB LOAD alignment, signature state),
  executes the publish skills' certification batteries per app ×
  channel, drives the D-118 in-app MCP servers as a modern-era
  client, signs only under a fresh single-use Owner-signed
  manifest, and assembles release-prep bundles. Portal submission
  is never performed.
- **Provenance:** born 2026-10-27 under D-120/D-121/D-136;
  rmcp/modern-only FROM BIRTH (D-113) — no legacy era has ever
  existed in this server.
- **Transport / era:** stdio; protocol 2026-07-28 only, guarded
  by the fleet byte-level era guard (WS1 template + W1 ordering
  amendment). serverInfo name `slipway` from the first commit.
- **Crate / binary:** package `slipway-mcp` 1.0.0 (the bare name
  `slipway` is taken on crates.io), binary `slipway`; edition
  2024; estate pin nightly-2026-09-25. Release binary (1.0.0,
  primo build of the landed tree) 8,138,720 B, sha256
  `622fe493eba06ebc91b27ca796ce0acd650857b90436eeafe7c78cb91c7e0539`.
- **Tools (15):** read — slipway_status, slipway_toolchain,
  slipway_inspect, slipway_verify, slipway_release_check,
  slipway_certify, slipway_bundle_check; execute — slipway_build,
  slipway_install, slipway_device_open, slipway_device_verify,
  slipway_device_close, slipway_drive_app; consequential —
  slipway_sign, slipway_release. Annotations from birth.
- **Home:** landed repo `https://github.com/qompassai/slipway`
  (main). The from-birth build tree was
  `~/workspace/slipway-build-20261027/`; the operational mirror
  on primo is `~/.local/share/slipway-mirror/` with state at
  `~/.local/share/slipway/`.
- **Methodology firewall:** configured forbidden roots
  (reference-material trees) are refused at config load on any
  overlap with a project root, and every artifact path is
  re-checked per call. Inputs are the estate's own apps only.
- **Gates at birth:** cargo test 18/18 (16 unit + 2 wire),
  clippy -D warnings clean, fmt clean, Diver gate 15/15 files
  diagnostics 0 (primo mirror, live config), four real
  certifications issued (light-show + ONTrack × Play + F-Droid),
  client QA 5/5 + 6/6 against both D-118 in-app servers, S3
  signing battery 5/5 against a labelled throwaway key.
- **Gates at 1.0.0 (2026-10-28):** re-gated on both estate
  machines — cargo test 18/18 (16 unit + 2 wire) on each,
  fmt clean, clippy `-D warnings` clean (resolved clippy
  0.1.100, f7575a9da8, pinned nightly-2026-09-25, both
  machines); no Rust source changed since the gated 0.1.0
  tree, so the birth Diver gate stands for the surface.
  Utilization: a scripted release-prep session against the
  1.0.0 binary (certify → bundle → bundle_check, both tamper
  legs refused/flagged) passed 12/12 — see UTILIZATION.md.
