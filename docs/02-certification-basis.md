# 02 – EASA Certification Strategy (CS-23)

> Engineering proposal, not an approved document. The certification basis,
> change classification and DAL must be agreed with the approving
> organisation (your DOA, or EASA) for each aircraft type.

## 1. Approval route

The CFB is fitted as a **change to the type design** of each aircraft.

| Route | When it fits | Notes |
|---|---|---|
| **Minor change** (Part 21 Subpart D, 21.A.91 classification), approved by a DOA under its privileges | Expected route: non-essential equipment, no interface to flight-critical systems, no effect on structure beyond a qualified mount, small electrical load | Fastest path. Good for a fleet of different CS-23 types: one design data package, type-specific installation drawings |
| **STC** (Part 21 Subpart E) | If the classification ends up "major" (e.g. new fuselage antenna cut-outs on a pressurised type, Level 4 aircraft, or integration with displays) | Approved Model List covers many types |
| **Standard Change** (CS-STAN) | Only for parts of the installation that an existing standard change covers | The CFB as a whole is not covered by a standard change today |
| **Part 21 Light** (Reg. (EU) 2022/1358) | Aircraft in its scope (certain light aircraft) | Simplified declaration-based process for some changes |

**Recommendation:** classify as a **minor change** and keep it minor. The
architecture is built for that: receive-only isolated interfaces, hardware RF
interlock, non-essential bus with its own circuit breaker, no penetration of
primary structure beyond approved antenna doublers.

## 2. Certification basis (applicable CS-23 Amendment 5+ items)

CS-23 Amdt 5 is performance-based; means of compliance are mainly ASTM
consensus standards accepted via AMC. Check the current amendment and the
aircraft's original type certification basis.

| Topic | Requirement | Means of compliance (proposed) |
|---|---|---|
| Equipment function and installation | CS 23.2500, 23.2505 | Installation drawings, ground test, flight test (EMI), ICA |
| System safety | CS 23.2510 | FHA + simple FMEA, ASTM F3230 / AMC 23.2510; see §3 |
| Electrical system, load | CS 23.2525 | Electrical load analysis; non-essential bus, load-shed |
| Electrical protection | CS 23.2525, wiring per ASTM F3313 / AC 43.13-1B practice | Dedicated CB, wire sizing, EWIS separation |
| Lightning / HIRF | CS 23.2520 | Non-essential: shows no adverse effect on other systems (DO-160G Sec. 20/21/22 as required) |
| Interference with other systems | CS 23.2500(b) | EMI ground + flight test against nav/com, ADF, GNSS, autopilot |
| Emergency landing loads (mounting) | CS 23.2270 | ASTM F3083 static load test or analysis of tray and box |
| Fire / flammability | CS 23.2325 | Enclosure in aluminium; flammability data for plastics, cables |
| Placards, AFM supplement | CS 23.2620, 23.2610 | AFMS: function, limitations, CB location |
| ICA | Part 21.A.61, CS-23 | ICA document: inspection, data load procedure, removal |
| Information security | AMC 20-42 (proportionate), ED-202A / ED-203A / ED-204A concepts | Security risk assessment, see §6 |

## 3. Safety assessment and software level

### Functional Hazard Assessment (summary)

| Failure condition | Effect | Classification (CS-23 Level 1–2) |
|---|---|---|
| Loss of all CFB functions | No tracking, no data upload; crew uses normal means | No Safety Effect |
| Misleading position report sent to ground | Wrong position on ops display; no crew action depends on it | No Safety Effect (Minor if used for SAR alerting) |
| Misleading data on cabin EFB (weather/text) | Crew may use outdated info; cross-check with approved sources required by AFMS | **Minor** |
| CFB causes EMI on nav/com | Possible effect on navigation | Prevented by design + EMI test; mitigation: CB pull (AFMS) |
| Cellular transmits in flight | Regulatory non-compliance, possible interference | Prevented by **hardware** interlock (WoW → W_DISABLE#) |
| CFB drives an avionics input | Could corrupt avionics data | **Not possible**: receive-only hardware, no transmit path |
| Electrical fault, smoke | Fire hazard | CB, fuse, wire sizing, enclosure, DO-160 Sec. 16 |

### Development Assurance Level

* Worst credible software failure condition: **Minor** → per AMC 23.2510 /
  ASTM F3230 (and FAA AC 23.1309-1E for comparison), **DAL D** for all
  CS-23 classes.
* **Proposed:** application software (`cfb-core`, `cfbd`) developed to
  **ED-12C / DO-178C Level D** (AMC 20-115D). The Linux base is treated as
  previously developed / COTS software with a service history and
  partitioning argument; the safety-relevant protections do not depend on it.
* If the FHA for a given installation shows *No Safety Effect* only, the
  software is Level E and the DAL D process is kept anyway as company
  practice (cheap at this size, and it keeps the option for EFB-related
  functions).

### What DAL D needs, and where it is in this repository

| ED-12C Level D objective | Artefact |
|---|---|
| Plans (PSAC, SDP, SVP, SCMP, SQAP) | To be written with the DOA (not yet in this repository) |
| High-level requirements | `CFB-HLR-xxx` tags in `cfb-core` docs + `docs/04-software-architecture.md` |
| HLR-based testing, normal + robustness | `cargo test` (unit tests in each module), simulation run in CI |
| Traceability HLR ↔ tests | Test names and HLR tags; generate trace matrix in CI |
| Configuration management | Git, signed tags, Software Configuration Index per release, ASCI |
| Quality assurance | Release checklist, independent review of release records |
| No structural coverage required at Level D | (statement coverage is still measured, as good practice) |

Airborne electronic hardware: CM5 / Pi 5 is COTS; per AMC 20-152A the
hardware is handled as COTS with a DAL D-proportionate argument (simple
functions, service history, environmental qualification).

## 4. Environmental qualification (ED-14G / DO-160G)

The Raspberry Pi is **not** qualified by its maker. Qualification is done on
the complete CFB (enclosure + carrier + power supply). Proposed categories for
a cabin/avionics-bay installation in an unpressurised or pressurised CS-23
aeroplane:

| Section | Test | Proposed category |
|---|---|---|
| 4 | Temperature and altitude | A2 / B2 (verify against installation zone) |
| 5 | Temperature variation | B |
| 6 | Humidity | A |
| 7 | Operational shock and crash safety | B |
| 8 | Vibration | S, curve M (fixed-wing reciprocating/turboprop) |
| 15 | Magnetic effect | Z (compass safe distance on label) |
| 16 | Power input | A (14/28 V), or Z incl. engine-crank dropout |
| 17 | Voltage spike | A |
| 18 | Audio frequency susceptibility | Z |
| 19 | Induced signal susceptibility | ZC |
| 20 | RF susceptibility | T / R (non-essential) |
| 21 | Emission of RF energy | **M** (cabin) – essential: the box has a fast SoC and radios |
| 22 | Lightning induced transients | only if wiring runs to external antennas in exposed zones |
| 25 | ESD | A |

## 5. Radios and spectrum

* **Iridium terminal**: type-approved Iridium Certus / SBD transceiver;
  antenna installation is part of the change (doubler, ground plane, sealing).
* **LEO broadband**: only with a terminal and antenna that has an
  installation approval for the aircraft type. Disabled by default.
* **Cellular**: the operator's terrestrial network may not be used in the
  air. The modem's RF is hard-disabled by the WoW interlock in flight.
  (Mobile communication on board through an approved onboard system under
  the EU MCA framework is a different system and out of scope.)
* **Cabin Wi-Fi AP**: transmitting PED-like source inside the aircraft;
  show tolerance of aircraft systems (ED-130A / DO-307A approach, plus flight
  EMI test). Can be configured off.
* National aircraft radio station licence must list the transmitters.

## 6. Information security

Proportionate application of AMC 20-42 for a non-essential connected system:

* **No inbound connections.** All ground traffic is outbound, mutually
  authenticated (WireGuard or TLS 1.3 with device certificates).
* **No path to avionics**: receive-only, galvanically isolated interfaces –
  the main security argument is the same as the safety argument.
* **Signed everything**: secure boot (BCM2712 customer key), dm-verity
  read-only root, RAUC bundles signed (X.509), manifests + ASCI signed
  (Ed25519) by the design organisation.
* Cabin Wi-Fi is a separate network segment with no route to the
  maintenance functions.
* Maintenance port: physical key switch + local technician credential.
* Organisation side: the DOA/CAMO handles the release keys under its
  Part-IS (Reg. (EU) 2023/203) information security management.

## 7. Continued airworthiness and software updates

See `05-update-process.md`. In short:

* Each released software is a **part number + version**, listed in the
  **Approved Software Configuration Index (ASCI)** issued under the DOA's
  change procedure.
* A software change is classified like any design change. Because the
  safety protections are in hardware, typical software changes (new message
  type, link driver, bug fix) are **minor changes** approved by the DOA
  under its privileges.
* The data load is a maintenance task described in the ICA, recorded in
  the aircraft records, with release to service per Part-ML / Part-M /
  Part-145 rules applicable to the aircraft.
* Ground-side content (map tiles, ground configuration) is not part of the
  airborne software and is outside the ASCI.
