# Utilization — slipway 1.0.0 (D-116 evidence)

Slipway's consumer workflow is release preparation: certify an
artifact, assemble the release bundle, and re-verify the bundle
before the Owner stamps and submits it. The session below runs
that flow for real against the 1.0.0 binary — a scripted
modern-era stdio client and a fixture artifact (a minimal
APK-shaped zip: manifest/dex/.so entries) in a scratch project,
including both tamper legs. The fixture is deliberately not a
real APK, so certification honestly returns `not_certified`
(its manifest cannot be read back); the release flow under test
— bundle assembly, bundle re-verification, and tamper refusal —
is exercised end to end on the record that certification issued.

- Date: 2026-10-28 (night run, fleet-to-1.0 program G-W1)
- Binary: `slipway 1.0.0`, release build on primo of the landed
  1.0.0 tree (8,138,720 B, sha256
  `622fe493eba06ebc91b27ca796ce0acd650857b90436eeafe7c78cb91c7e0539`).
  The same driver also passed 12/12 against the sandbox build
  of the same tree.
- Client shape: `server/discover` first, per-request
  `_meta["io.modelcontextprotocol/protocolVersion"] =
  "2026-07-28"` on every call; no `initialize` is ever sent.
- Driver: a stdlib-only Python stdio client; the fixture,
  config, and state lived in a fresh temp dir, removed after
  the run. Nothing was stubbed or mocked: every call hit the
  release binary over stdio.

## Transcript summary (12/12 PASS)

| Step | Call | Result |
| --- | --- | --- |
| 1 | `server/discover` | `supportedVersions == ["2026-07-28"]` exactly |
| 2 | `server/discover` serverInfo | `{"name": "slipway", "version": "1.0.0"}` |
| 3 | `tools/list` | the 15 tools, in class order (7 read, 6 execute, 2 consequential) |
| 4 | `slipway_status` | version `1.0.0`; the fixture project is the one configured project |
| 5 | `slipway_inspect` (fixture `app.apk`) | sha256 `cd7208d951caa33f47be1d9b78f85288690cf1e00d0776bcf621871afb72180c` — matches the fixture's locally computed hash; 3 zip entries censused |
| 6 | `slipway_verify` (`expect: "any"`) | verdict `verified`; signature state measured `unsigned` |
| 7 | `slipway_certify` (`google-play`) | verdict `not_certified` on the fixture, with measured per-gate evidence: manifest_readback=fail (fixture manifest is not a real Android manifest), target_sdk_36=unverified, alignment_16k=pass, signature_state=owner_action, listing_images_no_alpha=unverified, closed_test_conditional=conditional, developer_verification=owner_action; a certification record was issued and receipted |
| 8 | `slipway_release` (with step 7's record) | bundle `fixture-app-google-play-20261029T062241` assembled: 4 files (artifact, certification.json, receipts.jsonl, RELEASE-NOTES.md) under a `slipway.bundle/v1` manifest; `owner_stamp_required: true` (verdict not_certified) |
| 9 | `slipway_bundle_check` (step 8's bundle) | `valid: true` — all 4 file hashes match the bundle manifest and the artifact hash still agrees with the certification record |
| 10 | TAMPER leg 1: one byte appended to the artifact, `slipway_release` retried with step 7's record | refused with the typed domain error `certification_mismatch` (`isError: true`) — the S4 tamper-refusal behaviour, re-proven at 1.0.0 |
| 11 | TAMPER leg 2: one byte appended to the bundle's artifact copy, `slipway_bundle_check` re-run | `valid: false` — `app.apk: hash mismatch` reported against the bundle manifest |
| 12 | legacy probe: `initialize` with `protocolVersion: "2025-06-18"`, no `_meta` | refused by the era guard with typed error `-32022` "Unsupported protocol version", `supported: ["2026-07-28"]` |

Driver exit code 0 (`RESULT: ALL PASS`) on both runs (primo
release binary; sandbox build of the same tree).
