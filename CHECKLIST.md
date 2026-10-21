---
repo: slipway
updated: 2026-10-21
---

# slipway — master checklist

Derived from the slipway program documents (estate package
`slipway-20261021/`: STUDY.md, PROGRAM.md; the SB-1 workflow is
mirrored in the fathom Android wing phase document). Every item
traces to a stage gate in those documents. States: `[x]`
gated-closed · `[~]` delivered, ungated · `[ ]` open. The animated
tracker (assets/tracker.svg) is generated from this file only; it
moves only when an item's state changes at a gate.

This package authorizes no build: stages run only on the Owner's
build word, stage by stage.

## Ideation & design

- [x] Split ruled: fathom censuses reference artifacts, slipway builds/signs/verifies/installs/releases the Owner's own apps — two separate servers (Owner, 2026-10-21)
- [~] STUDY delivered (ruling record, methodology firewall, key-custody design, safety constitution) — spot-verified; full Pax gate still owed
- [~] PROGRAM delivered (stages S0–S5 with entry gates, acceptance, stop conditions) — spot-verified; full Pax gate still owed
- [~] SB-1 study-bridge handoff codified (study → bridge → build → census-back → two-server conformance diff)
- [x] GitHub repo created with master checklist + animated tracker (Owner's word, 2026-10-21; Q-SLIPWAY-1)

## S0 — census & pin-down (no server code)

- [ ] Pilot customer chosen (Q-SLIPWAY-3; recommendation on record: light-show)
- [ ] Pilot pipeline run by hand once, end to end, on primo (JDK/Gradle/AGP/SDK versions, build commands, signing arrangement, install path, release mechanics — evidenced on the machine)
- [ ] slipway.receipt v1 schema specified; STUDY UNDETERMINED items verified or carried forward explicitly

## S1 — read core

- [ ] Read tools landed (slipway_status, slipway_toolchain, slipway_verify, slipway_release_check) + receipt log; config roots canonicalized and disjoint, firewall checked at startup
- [ ] Inspector/conformance pass; SERVERS.md entry lands with this stage
- [ ] Verify proven against the S0 hand-built artifact (fingerprint match); refusal tests pass (reference-store path refused, overlap config refused, unresolvable key reference = structured error)

## S2 — build + install (unsigned only)

- [ ] slipway_build landed (bounded; input/output hashes receipted; debug-key auto-signing disabled or output classified debug-only — nothing in S2 signs)
- [ ] slipway_install proven on the named target class (Q-SLIPWAY-6)
- [ ] SB-1 census-back: slipway-built artifact censused by fathom's wing and diffed against the S0 hand-built artifact; build failure receipted honestly, never retried in a storm

## S2b — device verify

- [ ] Session surface landed (slipway_device_open / slipway_device_verify / slipway_device_close; a session takes an artifact hash from slipway's own receipt log, never a path; own builds only)
- [ ] Acceptance: S2 artifact verified end to end (install, launch, declared checks); a deliberately broken build fails verification with the failure receipted
- [ ] Emulator-class evidence settled (UNDETERMINED until an emulator is proven on primo; physical-device evidence does not stand in for it)

## S3 — signing (consequential class)

- [ ] Entry conditions met: signed-authority chain live (flowguard decide() + Volta key verification); Q-SLIPWAY-4 (key custody) and Q-SLIPWAY-7 (manifest gating) ruled
- [ ] slipway_sign landed (key by reference only; one canonical signed manifest per action; freshness fail-closed; nonce consumed before dispatch; decide() re-run at dispatch; signed output verified before release eligibility)
- [ ] Adversarial battery passed (stale manifest refused, replayed nonce refused, hash mismatch refused, flowguard block honored as final, no key material in any log/receipt/result — grep-proven)

## S4 — release prep (Owner-stamped)

- [ ] slipway_release assembles the bundle (signed artifact, fathom census, SB-1 conformance diff where one exists, receipts, release notes) under one signed manifest; slipway_release_check passes on the bundle before presentation
- [ ] One real pilot release prepared end to end and submitted by the Owner in the channel portal; live read-back matches the bundle hash record

## S5 — second customer

- [ ] Second customer onboarded by a fresh S0 pin-down for that project only (Owner's word; no project is onboarded by analogy)

## 1.0

- [ ] 1.0 — S0–S4 gated on the pilot customer: build → sign → verify → install → release-prep proven end to end
- [ ] 1.0 release record: registry record ai.qompass/slipway published on Owner's word; conformance entry current

## Beyond 1.0

- [ ] Maintenance: toolchain drift re-pin-downs per customer on a cadence
- [ ] Next-minor backlog (further customers and channels, each by fresh pin-down)
- [ ] Conformance re-probes at each release (declared vs live diff; per-release grades on a stated basis)
