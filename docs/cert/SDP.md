# Software Development Plan (SDP)

<sub>© 2026 Outthinkx by Luca De Simoni. All rights reserved. Proprietary, not open source. See [LICENSE](../../LICENSE).</sub>

> **DRAFT** for review with the design organisation (DOA). Items in
> ⟨angle brackets⟩ are decisions or data of the DOA / applicant. Not an
> approved document.

## 1. Standards

| Standard | Content |
|---|---|
| Requirements standard | One row per requirement in `docs/04`: ID `CFB-HLR-nnn` (never reused), one testable "shall", implementation reference, verification reference. Derived requirements marked as such and given to the safety assessment |
| Design standard | Layering and extension rules of `docs/12`: decisions only in `cfb-core` (no I/O, no `unsafe`), I/O only in `cfb-drivers`, wiring in `cfbd`; safety functions in hardware first |
| Coding standard | Rust edition 2021, toolchain pinned (`rust-toolchain.toml`); `#![forbid(unsafe_code)]` in airborne crates; `cargo clippy -D warnings` clean; no panics on external input (robustness tests); no unbounded buffers on external input; integer conversions explicit; comments state the requirement tag where a function implements one |

## 2. Environment

| Item | Choice |
|---|---|
| Language / compiler | Rust, pinned version (currently 1.97.0) |
| Build | cargo, `Cargo.lock` committed, reproducible build in CI |
| Target | aarch64 Linux (Raspberry Pi 5 / CM5) |
| Configuration management | git; `main` + release branches (SCMP) |
| CI | GitHub Actions: clippy, tests, simulations, arm64 build, evidence |

## 3. Process per change

1. Problem report or change request ⟨tracker⟩.
2. Requirement added or changed in `docs/04` (same commit as the code).
3. Code and tests; local gate `scripts/check.sh`.
4. Review (SVP §3) and merge to `main` only with green CI.
5. Trace matrix and SCI produced by CI for every commit.

## 4. Transition criteria

| From → to | Criterion |
|---|---|
| Requirement → code | Requirement reviewed, has an ID and a verification method |
| Code → merge | `scripts/check.sh` green, review done, docs updated |
| `main` → release | Trace matrix without problems, SCI from a clean tree, bench/hardware tests per `docs/10` recorded, SQA conformity review |
