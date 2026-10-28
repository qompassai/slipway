# Slipway — from-birth build report

**Commission:** Owner rulings D-120, D-121, D-136 (2026-10-27). Build
slipway, the estate's Android build/sign/verify/install/release MCP
server, from scratch — rmcp/modern-only **from birth** (D-113: no
legacy era ever exists in this server).
**Built:** 2026-10-27 (sandbox) + primo mirror gates, same day.
**Tree:** `~/workspace/slipway-build-20261027/` — fresh cargo
workspace, no commits (repo home is a later Owner word).
**Identity:** package `slipway-mcp` 0.1.0, binary `slipway`,
edition 2024, toolchain pin nightly-2026-09-25, rmcp **3.8.0**
(the fleet pin). Release binary **8,133,808 B**, sha256
`13d74a3b9f929d927f240bcd1515b188e019036ffc5bb3c31bce1d7b5181368b`.
serverInfo is `slipway` / 0.1.0 from the first commit, served in
the discover `_meta`, per the program's "serverInfo correct from
first commit" rule.
**Primo mirror:** `~/.local/share/slipway-mirror/` (source
checksum-verified against the sandbox tree; binary sha-verified),
state at `~/.local/share/slipway/`.

---

## 1. Stage gate table

| Stage | Gate | Result |
|---|---|---|
| S0 pin-down refresh | Pinned LightShow invocation (`play-build-aab.sh --live`, `JAVA_HOME=java-17`, `ANDROID_HOME=/opt/android-sdk`, cargo PATH) re-run by hand on primo | **PASS** — produced AAB sha256 `85a45253aa14ae7af4f303d323819eb4b682acdee8effc4ce5a35154b2798c78`, byte-identical to the 2026-10-24 pin-down (234,246,229 B, versionCode 2, targetSdk 36, unsigned by design) |
| S1 read core | inspect/verify against the real artifacts on primo (census, manifest read-back via bundletool/aapt2, signature measurement, per-.so alignment table) | **PASS** — exercised inside every certification below |
| S2 build + install | `slipway_build` light-show on primo; `slipway_install` to the emulator | **PASS** — build verdict `built-unsigned`, output hash `85a45253…` (the S0 artifact) in 4.7 s warm; installs receipted; note in §6 that a fully unsigned APK is not installable by Android design, so the device session ran on the S3-signed test copy |
| D-121 certification | Publish batteries executed for light-show + ONTrack × google-play + fdroid, verdicts with per-gate measured evidence | **ISSUED** — all four in §3. Every measurable gate passes for both apps on both channels; every verdict is `not_certified` solely on Owner-side items (§3) |
| D-121 client | Modern-era client (discover-first, no `initialize`) to both D-118 in-app servers, QA scenarios run | **PASS** — light-show 5/5 steps, ONTrack 6/6 steps, in the sandbox AND on primo; transcripts in `evidence/` |
| S3 signing machinery | Manifest-gated signing proven against a labelled throwaway key; refusal battery | **PASS 5/5** — §5. No real artifact signed; no real keystore exists for slipway |
| S4 release-prep | Bundle assembled for light-show/google-play; bundle re-verified; tampered artifact refused | **PASS** — bundle `light-show-google-play-20261028T012827`, `slipway_bundle_check` valid (4/4 file hashes, artifact↔certification agreement), `owner_stamp_required: true` (verdict not_certified); a one-byte-tampered artifact was refused with `certification_mismatch`. No portal submission, no uploads |
| S5 second customer | — | **OUT OF SCOPE** (per brief) |

### Standing gates (all on the final tree)

| Gate | Result |
|---|---|
| cargo test | **18/18** (16 unit + 2 wire integration). Split ≈ 50/50: validation (ELF/PNG parsers, config load, manifest verify, modern-frame forwarding, wire surface) / adversarial (initialize refused at 2025-06-18 **and** 2026-07-28, malformed frame −32700, unknown method −32601, missing arg −32602, firewall overlap refused, outside-root artifact refused, failed-gate ineligibility, nonce replay, stale manifest, tampered manifest) |
| clippy | `cargo clippy --all-targets -- -D warnings` — **clean** |
| fmt | `cargo fmt --check` — **clean** |
| Wire protocol | discover advertises exactly `["2026-07-28"]`; 15 tools in class order with annotations from birth; legacy `initialize` → **−32022** (guard-answered, `requested`/`supported` data correct); unknown method → −32601; malformed line → −32700 |
| Diver gate (primo mirror, live config, headless nvim) | **15/15 touched Rust files, rust-analyzer attached, diagnostics 0** (full run + re-gates after each fix round) |
| Inspector | **PARTIAL — see §6.5.** Inspector CLI 2.12.0 in ad-hoc `--cli` mode opens with `initialize` @2025-11-25 and does not fall back to discover against a modern-only server; reproduced identically against the fleet-gated eigenstyle and ardor binaries on both sandbox and primo, so the limitation is the Inspector's ad-hoc mode, not this server. The protocol assertions it would witness are covered by the wire tests, two foreign SDK clients (rmcp client; python driver), and manual refusal sessions |
| Methodology firewall | Config load refuses any project-root overlap with configured forbidden roots (`/home/phaedrus/.local/share/hypnosis-remake` on primo); every artifact path re-checked per call; `/etc/passwd` and `../` escapes refused live in the wire tests. Slipway's inputs were the estate's own apps only |

---

## 2. Decisions on the parked questions (Q1–Q7)

- **Q1 — repo home.** Build in `~/workspace/slipway-build-20261027/`
  as a fresh workspace; the registry/census entry is *prepared*
  (`docs/SERVERS-entry.md`) and lands on the Owner's word, per the
  brief. No repo created, nothing pushed or published.
- **Q2 — crate name.** `slipway-mcp` (binary `slipway`). The bare
  name `slipway` is taken on crates.io (checked live 2026-10-27:
  an unrelated deploy tool, v0.3.1); `slipway-mcp` was unclaimed
  (404). This is also the PROGRAM's own fallback precedent.
- **Q3 — pilot customer.** **Light-show** (Pax-ruled; also the
  package's recommendation and the only app with a complete S0
  pin-down).
- **Q4 — key custody.** Adopt-in-place / reference-only: slipway
  never holds keys. A project config names a keystore *path* and
  the *name of an environment variable* holding its password; the
  password travels only via the signer tool's own `env:` mechanism
  and never enters slipway's memory, logs, or reports. Custody
  changes are the Owner's ceremony. This build configured **no**
  real keystore anywhere.
- **Q5 — F-Droid boundary.** Slipway *executes* the fdroid-publish
  skill's battery — including running `fdroid-publish-check`
  itself — and issues verdicts; the signed APK stays the
  deliverable form and the fdroiddata MR stays the Owner's GitLab
  act (the skill's own "no credentials in this flow" posture).
- **Q6 — install target class.** Emulator only in this build
  (primo AVD `ontrack-pixel`, serial `emulator-5554`); non-emulator
  serials are refused with `target_out_of_scope`. Physical devices
  were not touched.
- **Q7 — manifest gating from S3 day one.** **YES** (Pax-ruled;
  the package's recommendation). No manifest, no signing — there
  is no ungated sign path in the tool surface at all.

Further design decisions taken in the build, with reasoning:

- **Tool surface = 15 tools** (Tiger MCP budget ≤ 25): read
  `slipway_status, slipway_toolchain, slipway_inspect,
  slipway_verify, slipway_release_check, slipway_certify,
  slipway_bundle_check`; execute `slipway_build, slipway_install,
  slipway_device_open, slipway_device_verify, slipway_device_close,
  slipway_drive_app`; consequential `slipway_sign, slipway_release`.
  Annotations (readOnly/destructive/idempotent/openWorld) on every
  tool from birth.
- **Device sessions open on hashes, not paths** (S2b model): the
  artifact must come from slipway's own receipt log for that
  project; a hash no receipt names is refused.
- **Sign-manifest machinery** (schema `slipway.sign-manifest/v1`):
  ed25519 signature over a canonical string, verified against
  configured pubkeys; freshness ≤ 60 s fail-closed both directions
  (skew 5 s); the nonce is consumed to append-only state **before**
  dispatch; the artifact hash must match; a hash marked ineligible
  by any failed gate can never be signed; the signed output must
  itself verify `signed` before its hash is release-eligible.
  Honest caveat: the estate's signed-authority chain (flowguard
  `decide()` + Volta) is not live for slipway — in this build the
  Owner-signed manifest **is** the gate.
- **Certification verdict rule:** `certified` only when *every*
  gate passes; `owner_action` / `conditional` / `unverified`
  results block exactly like failures but are reported distinctly,
  because they cannot be resolved by measurement.
- **Artifact facts are measured, never read from config:**
  targetSdk, versionCode, package, and permissions come from the
  artifact's own manifest (bundletool/aapt2); LOAD alignment and
  PNG color facts are parsed in pure Rust from the artifact bytes.

---

## 3. The four certification verdicts (D-121, in full)

Batteries executed on primo against the primo artifacts, by the
deployed binary. Gate definitions are the publish skills' own
(local-light-show-publish, local-ontrack-publish,
local-fdroid-publish); slipway executes and measures, it does not
fork definitions. Records on primo at
`~/.local/share/slipway/certifications/`; full JSON in
`evidence/primo/cert3-*.json` + `cert4-ot-play.json`.

### 3.1 light-show × google-play — NOT CERTIFIED

Artifact: `app-release.aab`, sha256 `85a45253…`, versionCode 2.

| Gate | Result | Measured evidence |
|---|---|---|
| manifest_readback | pass | manifest read back from the artifact via bundletool |
| target_sdk_36 | pass | targetSdkVersion read back 36 |
| alignment_16k | pass | all 2 64-bit .so LOAD alignments 0x4000 |
| package_identity | pass | manifest package = configured `ai.qompass.lightshow` |
| signature_state | **owner_action** | artifact is unsigned (jarsigner: no manifest) — signing is the Owner's step in this pipeline |
| listing_images_no_alpha | pass | 10 listing images measured 24-bit/no-alpha or JPEG (icons exempt) |
| data_safety_consistency | pass | zero manifest permissions; "no data collected/shared" posture consistent |
| closed_test_conditional | **conditional** | applies only if the Play account is personal + created after 2023-11-13; account type/date are not on record (Console-only) |
| developer_verification | **owner_action** | package + key registration under the Owner's verified developer identity not on record |

### 3.2 light-show × fdroid — NOT CERTIFIED (one Owner item)

| Gate | Result | Measured evidence |
|---|---|---|
| manifest_readback | pass | as above |
| target_sdk_36 | pass | 36 |
| alignment_16k | pass | all 2 at 0x4000 |
| fdroid_recipe | pass | License present, 1 full-SHA Builds pin, UpdateCheckMode; the ruled channel-split pin `4e1711da…` is among the recipe's commits |
| fastlane_tree | pass | tree complete for versionCode 2 (title ≤50, short ≤80 single-line, full ≤4000, icon 512×512, `changelogs/2.txt`) |
| listing_images_no_alpha | pass | 10 images measured |
| fdroid_publish_check | pass | the skill's own checker, run by slipway: exit 0, "appid ai.qompass.lightshow: READY" |
| foss_license | pass | Apache-2.0 LICENSE at repo root |
| developer_verification | **owner_action** | as above |

### 3.3 ontrack × google-play — NOT CERTIFIED

Artifact: `app-release.aab` (versionCode 201).

| Gate | Result | Measured evidence |
|---|---|---|
| manifest_readback | pass | bundletool read-back |
| target_sdk_36 | pass | 36 |
| alignment_16k | pass | the 1 64-bit .so at 0x4000 |
| package_identity | pass | `ai.qompass.ontrack` |
| signature_state | pass | artifact **is** signed — META-INF/ONTRACK-.SF/.RSA present, jarsigner "jar verified" (see §6.3) |
| listing_images_no_alpha | pass | 11 images measured across playstore/ + fastlane trees |
| data_safety_consistency | pass | 6 manifest permissions; `playstore/google-play.json` declares 5 collected data types (found under `privacy.dataSafety`) |
| closed_test_conditional | **conditional** | as §3.1 |
| developer_verification | **owner_action** | as §3.1 |

### 3.4 ontrack × fdroid — NOT CERTIFIED (one Owner item)

| Gate | Result | Measured evidence |
|---|---|---|
| manifest_readback | pass | as above |
| target_sdk_36 | pass | 36 |
| alignment_16k | pass | 0x4000 |
| fdroid_recipe | pass | License, full-SHA pin, UpdateCheckMode present |
| fastlane_tree | pass | complete for versionCode 201 incl. `changelogs/201.txt` |
| listing_images_no_alpha | pass | 11 images measured |
| fdroid_publish_check | pass | exit 0, "appid ai.qompass.ontrack: READY" |
| foss_license | pass | Apache-2.0 at root |
| developer_verification | **owner_action** | as above |

**Reading of the four:** both apps are *measurement-clean* on both
channels today. Nothing in any verdict is blocked by a defect in
an app or a tree; the blocks are the Owner's signing step
(light-show Play), the Play account-type fact (Console-only), and
the 2027 developer-verification registration. That is the honest
end-state of a certification whose standard is "every gate
passes."

### 3.5 Adversarial certification fixtures (both refused correctly)

- **Alpha plant:** a valid 2×2 RGBA PNG (color type 6) planted in
  a copied listing tree → `listing_images_no_alpha` **FAIL**
  ("1 listing image(s) break the no-alpha rule"), verdict
  not_certified, everything else unchanged.
- **Misaligned .so:** a default-page-size .so (LOAD align
  0x1000, compiled on primo) injected into a copy of the ONTrack
  APK → `alignment_16k` **FAIL**; package identity still measured
  and passed, proving the failure is the alignment gate itself.
- **Tampered bundle:** one appended byte on a certified artifact
  copy → `slipway_release` refused, `certification_mismatch`.

---

## 4. D-121 client evidence (drive_app)

Slipway as a modern-era MCP **client**: rmcp client side,
`ClientLifecycleMode::Discover` with
`preferred_versions: [2026-07-28]` — `initialize` is never sent.
Scenarios are the apps' own QA contracts. Run in the sandbox and
again on primo against the primo-deployed binaries; transcripts
in `evidence/client-*.json` and `evidence/primo/client-primo-*.json`.

- **light-show (osp_mcp, 8 tools) — 5/5 steps.** Discover
  modern-only; tools listed; 2 levels listed; load w1l1 →
  apply_move → step_sim returned a link-budget evaluation whose
  `received_dbm` matches the D-118 osp_sim figure (0.685 dBm);
  two `run_playtest` calls byte-identical (deterministic hashes).
- **ONTrack (ontrack-mcp, 6 tools) — 6/6 steps.** Discover
  modern-only; `app_status` reports 2026-07-28;
  `optimize_route` on the 1°-grid fixture twice byte-identical,
  total_cost **333567.84 m** (the D-118 fixture value), route
  starts at the depot and visits all four stops.

Every drive_app run appends a `device_session` receipt with the
scenario gate result (D-116 utilization evidence).

---

## 5. S3 signing proof (throwaway key only)

Test material lives in the primo fixture tree
(`~/.local/share/slipway-mirror/fixture/`), labelled
`THROWAWAY-test.jks` (RSA 2048, DN "CN=THROWAWAY slipway S3 test
key"); the manifest key is a labelled throwaway ed25519 seed.
The signed object was a **copy** of the ONTrack unsigned APK in
the fixture tree — no real artifact was signed, and no real
keystore exists.

| Case | Result |
|---|---|
| Valid fresh manifest | **signed** — output `unsigned-copy.signed.apk`, hash `89e482b6…`; independently verified: apksigner shows the throwaway DN; slipway's own post-sign gate required `signed` before eligibility |
| Replay of the same manifest | refused — `nonce_replayed` |
| Stale manifest (issued −3600 s) | refused — `manifest_stale` (60 s bound) |
| Manifest naming a different artifact hash | refused — `manifest_hash_mismatch` |
| Tampered manifest signature | refused — `manifest_signature_refused` |

The manifests were produced by an independent pure-python
RFC 8032 implementation (`tools/make_manifest.py`) and verified
by the Rust ed25519-dalek verifier — a cross-implementation
check, not a self-agreement.

**Device battery on the signed copy** (emulator-5554, AVD
`ontrack-pixel`): install ✓ (after removing the D-118 debug-signed
install — signature mismatch is Android's own rule working);
device_open by receipt hash ✓; device_verify **failed —
correctly**: the ONTrack release build native-crashes on launch
on this emulator (§6.4), flagged by both `process_alive` and the
tombstone-aware `no_fatal_crash` check. Positive path proven on
the Ardor QA companion APK (an estate test artifact): install →
open → verify **verified** (all four checks) → uninstall →
verify **failed** (installed=false) → close. Both sessions
closed; every step receipted.

---

## 6. Findings and corrections made during the build

1. **jarsigner exit code proves nothing (gate-integrity bug,
   caught by the first live certification).** The first AAB
   signature measurement trusted `jarsigner -verify` exit 0; it
   reported the by-design-unsigned LightShow AAB as *signed*
   (jarsigner exits 0 on unsigned jars, printing "jar is
   unsigned" / "no manifest."). Fixed: `signed` requires the
   positive "jar verified" marker and the absence of both
   unsigned markers. The re-run flipped the gate to the true
   `owner_action`.
2. **Fastlane root conflation.** The fastlane gate initially
   derived the metadata root from `listing_dir` (an *images*
   directory) and reported complete trees as missing. Fixed with
   an explicit `fastlane_dir` config field; both real trees then
   measured complete, and the skill's own `fdroid-publish-check`
   independently says READY for both apps.
3. **ONTrack's AAB is signed — contrary to the publish skill's
   standing note** that its canonical keystore "was never
   created". Measurement: `META-INF/ONTRACK-.SF/.RSA` present,
   jarsigner "jar verified". Some key signed the Oct 13 build;
   which one is an Owner-side fact to reconcile. (Flagged, not
   fixed — custody is the Owner's.)
4. **ONTrack's release APK native-crashes on launch** on the
   API 36 emulator (tombstone in logcat, process died; the D-118
   battery had exercised the *debug* build). Slipway's
   device_verify caught it once its crash check learned the
   native-crash signature (a Java-only "FATAL EXCEPTION" pattern
   missed it on the first pass — also fixed, and re-verified).
   The app defect itself is ONTrack's, not slipway's; it is
   recorded here as the device gate's first real catch.
5. **Inspector ad-hoc mode (gate limitation, §1).** Inspector
   CLI 2.12.0 `--cli` opens with legacy `initialize` and gives up
   on the designed −32022; identical against eigenstyle and
   ardor, on both hosts. Reported as partial rather than
   papered over; the WS gates' exit-0 Inspector runs are not
   reproducible from the recorded commands.
6. **Build-gate message fix.** The first `slipway_build` failure
   (a misconfigured second output path — the Play build produces
   no APK in that tree) surfaced the build script's trailing
   NOTE line as if it were the error. The tool now names the
   failed output gates when the command itself ran clean; the
   config was corrected to the AAB as the build's sole output.
7. **Data Safety declaration shape.** ONTrack's
   `google-play.json` is comment-headed JSONC nesting the
   declaration at `privacy.dataSafety.dataCollected`; the gate
   now strips comment lines and searches recursively (bounded)
   rather than demanding one fixed shape.
8. **Caveat — ineligibility scope.** A failed *device_verify*
   marks the artifact hash ineligible for signing, like any
   failed gate; but device failures can be environmental (the
   uninstall negative above is the proof case, run under an
   isolated state dir for exactly this reason). Recommendation
   for the landing pass: scope permanent ineligibility to
   artifact-intrinsic gates (signature, alignment, census) and
   record device/session failures as session-scoped. Behavior
   was *not* changed mid-build; the recommendation is recorded
   here for the Owner's ruling.

---

## 7. Edition / toolchain / dependency audit (D-111)

- Edition **2024** throughout; pin **nightly-2026-09-25**
  (rustc 1.100.0-nightly); the primo mirror also builds under
  primo's cargo 1.99.0 — the Diver gate ran against the mirror.
- Direct dependencies (9): rmcp 3.8.0 (fleet pin; server +
  client + child-process transport), serde, serde_json, sha2,
  tokio, toml, zip (deflate only), ed25519-dalek 2, chrono.
- **No `*-sys`, no FFI, no C dependencies:** full tree is 190
  crates, zero `-sys`/openssl/ring hits (`cargo tree` grep).
- Source: 4,808 lines (src + tests), 15 library modules + bin.
- `#![forbid(unsafe_code)]` in the library.

## 8. Owner-gated remainders (named, not silently dropped)

1. **Repo home + publication:** GitHub repo creation for
   slipway, crates.io publish (name `slipway-mcp` is unclaimed),
   MCP Registry record (prepared text in `docs/SERVERS-entry.md`),
   MCPB bundle — all await the Owner's word. Nothing was
   committed or pushed anywhere by this build.
2. **Real signing:** a real keystore + the Owner's manifest
   pubkey in config. Until then `slipway_sign` correctly refuses
   every production project (`key_unavailable` / no trusted
   pubkeys configured).
3. **Portal facts only the Owner can supply:** Play account
   type + creation date (decides the closed-test conditional);
   developer-verification registration (decides that gate on all
   four certifications); ONTrack's actual signing-key provenance
   (§6.3).
4. **Portal submission itself:** permanently the Owner's act by
   design; S4 produces the bundle as the deliverable.
5. **S5** (second customer build-out) — out of scope here.
6. **Ineligibility-scope ruling** (§6.8) and the Inspector
   session-mode question (§6.5) if the fleet wants Inspector
   evidence in its recorded form.

## 9. Evidence index

- `evidence/client-light-show.json`, `evidence/client-ontrack.json`
  — sandbox client transcripts; `evidence/primo/client-primo-*.json`
  — primo repeats.
- `evidence/primo/cert3-*.json`, `cert4-ot-play.json` — the four
  final certification outputs (cert1/cert2 runs preserved —
  they are the §6.1/§6.2 bug evidence).
- `evidence/primo/device*.json` — install/open/verify/close
  transcripts incl. the negative and the ONTrack crash catch.
- `evidence/primo/release-*.json` — bundle, bundle check, tamper
  refusal.
- `evidence/inspector/` — Inspector attempts (sandbox).
- Primo state: receipts at `~/.local/share/slipway/receipts.jsonl`
  (every action above), certification records at
  `~/.local/share/slipway/certifications/`, the S4 bundle at
  `~/.local/share/slipway/bundles/light-show-google-play-20261028T012827/`.
