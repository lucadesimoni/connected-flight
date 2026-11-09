# Functional Hazard Assessment – Connected Flight Box

<sub>© 2026 Outthinkx by Luca De Simoni. All rights reserved. Proprietary, not open source. See [LICENSE](../../LICENSE).</sub>

> **DRAFT** for review with the design organisation (DOA). Items in
> ⟨angle brackets⟩ are decisions or data of the DOA / applicant. Not an
> approved document.

Reference: CS 23.2510 / AMC 23.2510 with ASTM F3230 (FAA AC 23.1309-1E for
comparison). Aircraft classes considered: CS-23 Level 1–4 (incl. PC-12 ≈
Level 3 / Class III, PC-24 ≈ Level 4 / Class IV, see `docs/07`).
Two configurations: **portable** (no aircraft connection) and **installed**
(receive-only interfaces, hardware RF interlock).

## 1. Functions

| ID | Function | Users of the output |
|---|---|---|
| F1 | Determine flight phase | the box itself (radio permission, update gate) |
| F2 | Transmit position reports and events | ground operations / maintenance |
| F3 | Upload flight / engine data after landing | maintenance, engine programmes |
| F4 | Edge apps (exceedance, trend, summary) | maintenance / operations on ground |
| F5 | Receive aircraft data (ARINC 429, serial, CAN) | F1–F4 |
| F6 | Own sensors (GNSS, IMU, barometer) | F1–F4 |
| F7 | Field-loadable software update | maintenance |
| F8 | Status lights / maintenance page | crew (status of the box only), technicians |
| F9 | Cabin Wi-Fi access point (optional) | crew EFB |

## 2. Failure conditions

| ID | Failure condition | Phase | Effect on aircraft / crew | Classification | Mitigation (design) |
|---|---|---|---|---|---|
| FC1 | Loss of all CFB functions | all | None; ground loses tracking/data | No Safety Effect | – |
| FC2 | Erroneous position report / event to ground | flight | Ground sees wrong data; no crew action | No Safety Effect (Minor if used for SAR alerting – ⟨operator use to confirm⟩) | GNSS/ARINC cross-check, source shown in data |
| FC3 | Cellular transmits in flight | flight | Possible interference, regulatory non-compliance | ⟨Minor⟩ | **Installed:** hardware WoW interlock (W_DISABLE#); software inhibit + re-assert; **portable:** radio switch + software inhibit + procedure (modem unplugged) |
| FC4 | CFB drives or loads an avionics bus | all | Corrupted data to other receivers | ⟨Major if possible⟩ → **designed out** | Receive-only hardware (no transmitter fitted), high-impedance isolated receivers, bus-load check per installation |
| FC5 | EMI from the CFB affects nav/com/autopilot | all | Degraded navigation or communication | ⟨Minor–Major depending on system⟩ | DO-160G Sec. 21 Cat M, EMI ground/flight test, CB pull procedure in AFMS |
| FC6 | Electrical fault, smoke, fire | all | Smoke in cabin / bay | ⟨Major⟩ → mitigated | Dedicated CB, wire sizing, aluminium enclosure, no lithium battery (installed), DO-160 Sec. 16 |
| FC7 | Software update installed in flight / corrupt update | flight / ground | Box restart in flight; box inoperative | No Safety Effect | Update gate (parked, engine off, maint mode), A/B rollback, signatures |
| FC8 | Misleading information to crew via EFB (F9) | flight | Crew acts on outdated weather/text | **Minor** | AFMS: approved sources primary; data age shown by the EFB |
| FC9 | Misleading status light | flight | Crew believes radios are off while on | ⟨Minor⟩ | Lights driven from the same permission state as the radios; installed unit: interlock independent of lights |
| FC10 | Mechanical detachment (crash loads) | emergency landing | Loose object | ⟨Major⟩ → mitigated | Tray qualified to CS 23.2270 / ASTM F3083; portable: secured mount procedure |

## 3. Development Assurance Level

Worst failure condition to which **software** contributes: FC8/FC9 →
**Minor** → **DAL D** for all CS-23 classes (Minor → D in Class I–IV).
FC3 and FC4 are not attributed to software on the installed unit because
hardware prevents them; on the portable unit FC3 is mitigated by the radio
switch and procedure, ⟨to be accepted by the DOA for the portable
configuration⟩.

Software level of airborne software (`cfb-core`, `cfb-drivers`, `cfbd`,
`cfb-io`, `cfb-maint`): **Level D** (ED-12C / DO-178C, AMC 20-115D).
Ground software (`cfb-ground`) is not airborne software; it follows the
same quality gates without certification credit.

## 4. Derived safety requirements (to the hardware / installation)

| ID | Requirement |
|---|---|
| DSR1 | The cellular modem shall be unable to transmit when WoW indicates air, independent of software (installed unit) |
| DSR2 | No interface to an aircraft system shall contain a transmitter / driver |
| DSR3 | Interfaces to aircraft signals shall be galvanically isolated |
| DSR4 | The CFB shall be on a non-essential bus with its own circuit breaker |
| DSR5 | Emissions shall meet DO-160G Sec. 21 Cat M (or as agreed) |
| DSR6 | The AFMS shall state that CFB/EFB data is supplementary |
