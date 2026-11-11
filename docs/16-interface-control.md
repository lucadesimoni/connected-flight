# 16 – Interface Control Document (ICD) – Connected Flight Box

<sub>© 2026 Outthinkx by Luca De Simoni. All rights reserved. Proprietary, not open source. See [LICENSE](../LICENSE).</sub>

> Issue 0.1 (proposal). The interfaces of the **integrated unit** (CFB-HW-100),
> for the OEM's installation and systems engineering. Pin assignments and
> electrical values are a **proposal, to be frozen** with the OEM before
> the carrier board layout (rev B). Software and data interfaces are
> implemented and versioned (docs/12 §3).

## 1. Unit overview

| Item | Value |
|---|---|
| Function | Air–ground connectivity, store-and-forward, edge processing; non-essential |
| Hardware part | CFB-HW-100 (base + modules, docs/03, docs/11 §B) |
| Software part | CFB-SW-nnnn, field-loadable (docs/05) |
| Mass (target) | ≤ 0.9 kg without antennas |
| Envelope (target) | ≈ 180 × 120 × 50 mm, tray-mounted, conduction-cooled, no fan |
| Environment | DO-160G categories per docs/02 §4 (to be agreed per installation zone) |

## 2. Electrical interfaces

### 2.1 Connectors (proposal)

| Ref | Type | Function |
|---|---|---|
| J1 | MIL-DTL-38999 Series III, 13-35 shell | Power, discrete inputs |
| J2 | MIL-DTL-38999 Series III, 15-35 shell | ARINC 429 RX ×4, RS-422/232 RX ×2, CAN ×1 |
| J3 | M12 X-coded or 38999 Ethernet insert | Maintenance Ethernet (keyed / covered) |
| J4 | M12 X-coded | LEO terminal Ethernet (option) |
| J5–J8 | TNC female | GNSS (active, 5 V bias), cellular main, cellular diversity, Iridium |

### 2.2 J1 power and discretes (proposal)

| Pin | Signal | Characteristics |
|---|---|---|
| A, B | +28 VDC / +14 VDC in | 9–36 V, DO-160G Sec. 16; ~0.2 A typ. at 28 V, 0.6 A peak; 3 A CB on a non-essential bus |
| C, D | Power return | |
| E | Chassis / shield | |
| F | WoW (ground = active) | Opto-isolated, ground/open (selectable 28 V/open by strap); feeds the **hardware RF interlock** and software |
| G | Engine running (oil pressure switch) | Opto-isolated, ground/open |
| H | Maintenance mode key | Opto-isolated, ground/open |
| J | Spare discrete | Opto-isolated |
| K | Discrete return | Isolated return |
| L | Annunciator out (optional "CFB FAULT") | Open-drain, isolated, 28 V / 100 mA; the only output, to a dedicated lamp, never to an avionics input |

### 2.3 J2 data inputs (receive-only)

| Pins | Signal | Characteristics |
|---|---|---|
| ARINC 429 RX 1–4 (A/B each) | ARINC 429 receive | High-impedance receivers (HI-3593 class), low or high speed per channel; **no transmitter fitted**; counts as one receiver load per bus |
| RS-422 RX 1 (± pair) | Serial receive | Isolated; no TX driver fitted |
| RS-232 RX 2 | Serial receive | Isolated; no TX line connected |
| CAN H/L | CAN receive | Isolated; transceiver TXD forced recessive (listen-only) |

Label tables per installation from the OEM's avionics ICD → `[[arinc429.labels]]`
(channel, label, SDI, format, range, significant bits, parameter name).

### 2.4 RF

| Port | Band | Notes |
|---|---|---|
| GNSS | L1 (+L5 with NEO-F10N) | Active antenna, 5 V bias from the unit, TSO-C190 antenna |
| Cellular main / diversity | LTE bands of the EG25-G (or 5G module) | Transmits **only** with WoW = ground (hardware interlock) |
| Iridium | 1616–1626.5 MHz | Iridium 9603N / Certus per build |

## 3. Data interfaces (implemented)

| Interface | Definition | Version / compatibility |
|---|---|---|
| Air–ground frame | `crates/cfb-core/src/frame.rs`: `0xCF`, version nibble 1, type, seq, time (s since 2020), LEB128 length, payload, CRC-32C | Additive only; incompatible change → version 2, both accepted (docs/12) |
| Position report | 17-byte payload (frame.rs `PositionReport`) | as above |
| SBD batching | `[len u8][frame]…` ≤ 340 bytes MO / 270 MT (`cfb-core/src/sbd.rs`) | frozen |
| Ground ack | `Ack` frame listing received sequence numbers | frozen |
| Ground service API | `/api/boxes`, `/api/track`, `/api/events`; SBD webhook `POST /sbd` (`crates/cfb-ground`) | additive |
| Status snapshot | `/run/cfb/status.json` (`cfbd/src/status_out.rs`) | fields added, never renamed |
| Maintenance page | HTTP on the maintenance port; two token-protected actions | – |
| I/O line protocol (`cfb-io` → `cfbd`) | `crates/cfbd/src/input.rs` header | new tokens only |
| Installed configuration | `config/*.toml` schema (`cfb-core/src/config.rs`), signed, validated at load | new fields with defaults |
| Edge app package | TOML rule package, signed, in the ASCI (`cfb-core/src/edge.rs`, docs/06) | fields with defaults |
| Software update package | RAUC bundle + `manifest.toml` + ASCI, Ed25519 / X.509 (docs/05) | – |

## 4. Installation constraints

* Non-essential bus, own circuit breaker; placard; AFMS supplement.
* Antennas: GNSS and Iridium with sky view (top fuselage), cellular
  belly/fairing; separation from VHF/GNSS antennas per the installation's
  EMI analysis.
* ARINC buses: bus-load check per tapped bus (≤ 20 receivers per transmitter).
* Compass safe distance per DO-160G Sec. 15 result.
* Maintenance access to J3 and the status lights.

## 5. Open items to freeze with the OEM

1. Connector types and pinout (OEM standard parts list).
2. Discrete input levels per aircraft (28 V/open vs ground/open).
3. ARINC channel count and speeds per type; label tables.
4. Annunciator output: wanted or not.
5. Mechanical envelope, mounting tray, cooling path in the chosen zone.
6. DO-160G categories per installation zone.
