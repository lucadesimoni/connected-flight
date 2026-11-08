# 07 – Installation on Pilatus PC-12 and PC-24 (planning notes)

<sub>© 2026 Outthinkx by Luca De Simoni. All rights reserved. Proprietary, not open source. See [LICENSE](../LICENSE).</sub>

> Planning notes only. Each statement about the aircraft must be checked
> against the TCDS, the aircraft's wiring/ICD data and the TC holder.
> Nothing here is approved data.

## 1. Fit

| | PC-12 NG / NGX | PC-24 |
|---|---|---|
| Category | Single-engine turboprop, pressurised, up to 9 passengers | Twin-engine business jet, pressurised, commuter category |
| CS-23 Amdt 5 equivalent | ~Level 3 | Level 4 |
| AC 23.1309-1E class (for DAL reference) | Class III | Class IV |
| Avionics | Honeywell Primus Apex | Pilatus ACE (based on Honeywell Primus Epic) |
| Engines | P&WC PT6A-67P (NG) / PT6E-67XP (NGX) | 2 × Williams FJ44-4A |
| DAL for the CFB (Minor worst case) | **D** | **D** (Minor is D in every class; Major would be C) |

The CFB architecture (receive-only, hardware RF interlock, non-essential
bus, ground-only outputs) applies to both unchanged. What differs is the
installation, the certification basis and the data.

## 2. Certification basis

* Neither type was certified to CS-23 Amdt 5. The PC-12 basis goes back
  to FAR 23 / JAR-23; the PC-24 was certified to CS-23 (commuter) with
  special conditions. The exact basis is in each TCDS.
* A change is assessed under **21.A.101 (changed product rule)**: normally
  the aircraft's existing basis for the areas the change touches, plus any
  later requirements that the agency decides must apply. The compliance
  matrix in `02-certification-basis.md` (written in CS 23.2xxx terms) must be
  mapped to the corresponding paragraphs of the type's basis (e.g. 23.1301,
  23.1309, 23.1351, 23.1431, 23.1529).
* **Classification:** expect **minor** for a cabin/avionics-bay box
  using existing data buses and an existing approved antenna. New fuselage
  penetrations in the pressure vessel (Starlink, Iridium, cellular
  antennas) push towards **major / STC**, especially on the PC-24.
* **Route:**
  * For the TC holder: a design change or a factory/retrofit option under
    its DOA. This is the shortest path, and the data access and ICDs are
    in-house.
  * For a third party: an STC, with an Approved Model List per type and
    the data access agreed with the TC holder and the avionics OEM.

## 3. Data access (the critical item)

| Need | Source | Note |
|---|---|---|
| Which ARINC 429 buses carry nav, air data, engine data, and where they can be tapped | Pilatus / Honeywell wiring diagrams and ICDs | Proprietary. Without them, no installation |
| Label table (numbers, SDI, BNR/BCD, range, resolution, rate) | Same ICD | Goes into `[[arinc429.labels]]` in the aircraft's `cfb.toml`; the engine labels in `config/cfb-pc12-example.toml` are **illustrative** |
| Spare receiver capacity on each bus (≤ 20 receivers per transmitter) | LRU installation manuals | Check every tapped bus |
| Data only on the main avionics bus (ASCB in Epic-family systems), not on 429 | Honeywell | Needs either a 429 output configured on an LRU, or a new receive module. The modular design allows that, but it's a larger change |
| Engine data rights | P&WC / Williams, Pilatus | Trend/exceedance use may be covered by existing engine programmes |

**Avoid duplication.** Check what each type already has: an engine data
downlink, a factory connectivity option, or Iridium for voice/data. The
CFB can carry existing data sets, or use an existing approved satcom
terminal over Ethernet or serial, instead of adding a second one.

## 4. Antennas and links

| Link | Installation | Note |
|---|---|---|
| **Starlink (LEO)** | Approved Starlink aviation terminal + antenna for the type; CFB connects by Ethernet (satcom module) | The terminal and its antenna/radome need their own installation approval: check whether one exists for the type before planning. Largest single item of the change (structure, drag, power) |
| **Iridium Certus** | Top-fuselage antenna (or existing Iridium antenna through a splitter/second port if approved) | Global backup, works at all latitudes |
| **4G/5G** | Blade or conformal antenna; ground use only (hardware interlock) | Can share a fairing; check intermodulation with VHF/GNSS |
| **Wi-Fi** | Small cabin or belly antenna, ground use; optional cabin AP for EFB | PED tolerance / EMI flight test |
| **Own GNSS** | Separate top antenna preferred | A splitter on the aircraft GNSS antenna needs an approved type and GNSS performance check |

Over IP bearers (Starlink, Certus, cellular, Wi-Fi) the CFB runs one
WireGuard tunnel per bearer to the operator's ground service, with
end-to-end acks (`crates/cfbd/src/transport.rs`). Link health is measured
from what actually gets through, so obstructions and handovers are
handled automatically.

## 5. Environmental and electrical (vs. the generic CS-23 assumptions)

| Item | PC-12 | PC-24 |
|---|---|---|
| DO-160G temperature/altitude | Pressurised cabin or avionics bay category; confirm the zone | Same; avionics bay cooling and decompression case to altitude (box must survive, need not work) |
| HIRF / lightning | Non-essential: no adverse effect on other systems, but external antennas and wiring add exposure; DO-160 Sec. 20/21/22 as required | Level 4 / jet: expect higher HIRF environment levels and full Sec. 22 for antenna cables |
| Emissions (Sec. 21) | Cat M minimum | Cat M; check against the jet's sensitive receivers (GNSS, VHF, ADS-B, TCAS) |
| Electrical | 28 V DC, non-essential bus, own CB, load-shed | 28 V DC, non-essential bus, own CB, load-shed |
| Load analysis | ~0.2–0.6 A plus the Starlink terminal's own power | Same |
| Weight | box ~0.8 kg + antennas + Starlink terminal | same |

## 6. Configuration per aircraft

One software part number for both types. Per-aircraft data:

* `cfb.toml`: modules fitted, label table from the ICD, link policies and
  budgets, transport tunnel addresses, IMU mounting axes;
* edge app packages per type: a turboprop app for the PC-12 (torque, ITT,
  Ng, Np, fuel flow); a jet app for the PC-24 (N1, N2, ITT, oil pressure,
  per engine via SDI or label);
* ASCI entries for both software and apps.

## 7. Suggested sequence

1. Data agreement and ICDs with the TC holder / avionics OEM.
2. Bench: CFB on the avionics test bench, receiving the real buses. This
   validates the label table and checks the bus loading.
3. Pre-application meeting with the authority (classification, basis,
   Starlink terminal approval status).
4. Prototype installation on one aircraft: EMI ground test, flight test
   (all links, phase logic, no interference).
5. DO-160 qualification of the production unit (CM5 build).
6. Approval, ICA, AFM supplement; series retrofit with OTA from day one.
