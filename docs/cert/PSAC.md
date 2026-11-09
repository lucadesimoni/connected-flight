# Plan for Software Aspects of Certification (PSAC) – Connected Flight Box

<sub>© 2026 Outthinkx by Luca De Simoni. All rights reserved. Proprietary, not open source. See [LICENSE](../../LICENSE).</sub>

> **DRAFT** for review with the design organisation (DOA). Items in
> ⟨angle brackets⟩ are decisions or data of the DOA / applicant. Not an
> approved document.

## 1. System overview

The Connected Flight Box (CFB) is non-essential connectivity equipment for
CS-23 aeroplanes: in-flight position and event reporting over satellite,
ground upload of flight/engine data over cellular/Wi-Fi, on-board edge
analysis, and field-loadable software. It has no output to any aircraft
system (receive-only interfaces). See `docs/01`, `docs/03`.

## 2. Certification basis and approval route

⟨Per aircraft type: minor change (Part 21 Subpart D) or STC (Subpart E),
under the changed-product rule 21.A.101.⟩ Software: ED-12C / DO-178C as
recognised by AMC 20-115D. Airborne electronic hardware: COTS per
AMC 20-152A (DAL D). See `docs/02`, `docs/07`.

## 3. Software level

**Level D**, from the FHA ([FHA.md](FHA.md)): worst software-related
failure condition Minor; failure conditions that could be more severe
(cellular in flight, avionics bus interference) are prevented by hardware.

## 4. Software overview

| Component | Function | Level |
|---|---|---|
| `cfb-core` | Decisions: phase, link policy, queue, framing, ARINC/NMEA/IMU decoding, reference selection, edge rule interpreter, OTA state machine, update gate | D |
| `cfb-drivers` | I/O: link drivers (UDP/WireGuard, Iridium SBD), IIO sensors, serial, discretes, LEDs | D |
| `cfbd`, `cfb-io` | Wiring daemon, I/O adapter | D |
| `cfb-maint` | Maintenance page (separate process, no flight functions) | D |
| Linux, RAUC, WireGuard, NetworkManager, ModemManager | Operating environment | Previously developed / COTS software, ⟨service history and partitioning argument⟩ |
| Edge app packages | Data (rule sets) for the verified interpreter | Configuration data, ⟨classification per DOA⟩ |
| `cfb-ground` | Ground service | Not airborne |

## 5. Life cycle

Iterative development with per-change verification (`docs/12`):
requirements (HLR table `docs/04`) → code → tests per HLR → review →
merge to `main` only with green CI. Plans: [SDP](SDP.md), [SVP](SVP.md),
[SCMP](SCMP.md), [SQAP](SQAP.md).

## 6. Life-cycle data

| Data | Where |
|---|---|
| Plans | `docs/cert/` |
| High-level requirements | `docs/04` HLR table (`CFB-HLR-xxx`) |
| Design description | `docs/01`, `docs/03`, `docs/04`, `docs/12`, source documentation |
| Source code | repository, `crates/` |
| Verification cases and results | unit / integration / robustness tests; CI logs; trace matrix (`scripts/evidence.py`) |
| Configuration index | `scripts/sci.py` per build |
| Problem reports | ⟨tracker to be defined (e.g. GitHub issues with a fixed template)⟩ |
| SQA records | ⟨SQAP⟩ |
| Software Accomplishment Summary | at the end of the cycle |

## 7. Additional considerations

* **Tool qualification:** no tool output is used without verification
  (compiler output is verified by tests on the target; clippy and the trace
  script are verification aids) → no TQL required ⟨to confirm⟩.
* **Previously developed software:** Linux and system services; argument
  based on service history, the safety function being in hardware, and
  process isolation ⟨DOA⟩.
* **Field-loadable software:** part numbers, ASCI, signatures, update gate,
  A/B rollback (`docs/05`).
* **User-modifiable data:** none on the box; configuration and edge
  packages are signed and controlled by the ASCI.
* **Information security:** AMC 20-42 (proportionate), `docs/02` §6,
  `docs/13`.

## 8. Schedule and authority involvement

⟨Stages of involvement (SOI 1–4) and dates agreed with the authority / DOA.⟩
