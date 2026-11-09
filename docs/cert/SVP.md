# Software Verification Plan (SVP)

<sub>© 2026 Outthinkx by Luca De Simoni. All rights reserved. Proprietary, not open source. See [LICENSE](../../LICENSE).</sub>

> **DRAFT** for review with the design organisation (DOA). Items in
> ⟨angle brackets⟩ are decisions or data of the DOA / applicant. Not an
> approved document.

## 1. Methods

| Method | Applied to | Evidence |
|---|---|---|
| Requirements-based tests (normal range) | every HLR | unit/integration tests named in `docs/04` |
| Robustness tests | every parser of external data (CFB-HLR-110), abnormal inputs, input loss, timeouts | `tests/robustness.rs`; tests for timeouts, link loss, power fail |
| Simulation | complete flights (four scripted scenarios) | CI simulation runs |
| Integration on localhost | box chain ↔ ground service, maintenance page, SBD webhook | end-to-end procedures (`docs/10`) |
| Hardware/bench tests | radios, sensors, modem, interlock, power | `docs/10` T1–T28, recorded |
| Reviews | requirements, code, tests, plans | review records ⟨SQAP⟩ |
| Analysis | security / robustness (`docs/13`), resource use (memory/CPU limits in systemd units) | review reports |

Structural coverage is not required at Level D; statement coverage is
measured as good practice ⟨tool: cargo-llvm-cov, to be added to CI⟩.

## 2. Traceability

`scripts/evidence.py` builds the HLR → test matrix from `docs/04` and
the actual test run, for every commit, and fails if a referenced test is
missing or failed. Tests not referenced by any HLR are listed and reviewed
(possible missing requirement).

## 3. Reviews and independence

Level D has no independence requirements for most objectives ⟨confirm
per DO-178C Annex A table for the agreed objectives⟩. Practice: every
change reviewed by a second person before merge to a release branch;
release review by SQA (SQAP).

## 4. Regression

Full test suite, simulations and evidence on every push (CI). A release
re-runs the bench tests affected by the change (impact analysis recorded).
