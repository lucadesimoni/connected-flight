# 06 – Edge Apps (in-box computing)

<sub>© 2026 Outthinkx by Luca De Simoni. All rights reserved. Proprietary, not open source. See [LICENSE](../LICENSE).</sub>

## Why

A satcom link is expensive and slow, and raw data at 1–10 Hz is useless to
an operations desk anyway. The box computes the answer on board and sends
only that:

| Raw data | Edge result | Size |
|---|---|---|
| ITT at 1 Hz for a 3 h flight (~10 800 values) | `EXC ITT_HIGH START itt>800.0 val=812.0` + `END dur=13s peak=812.0` | ~90 B |
| Engine parameters for a whole cruise | one averaged stable-cruise snapshot (`TREND ENG_CRUISE …`) | ~150 B |
| Whole flight | `SUM block=29min flight=25min dist=60NM max_alt=8700ft max_nz=2.21 landings=1` | ~90 B |

High-priority results (exceedances) go out at once over satcom; trend and
summary wait for the cheapest link (Starlink in flight, or 5G / Wi-Fi on
ground). The raw flight data file still uploads on ground for full analysis.

## Containment: why apps don't affect certification of the box

| Rule | How it's enforced |
|---|---|
| Apps read a copy of the decoded data, nothing else | `DataView` is built per tick; apps have no handle on I/O |
| Apps can only produce messages | `EdgeApp::tick` returns `Vec<EdgeOutput>`; the runtime queues them |
| No access to radios, RF enables, update gate, avionics | Not reachable from the app interface (and avionics interfaces are receive-only in hardware anyway) |
| Bounded resources | Max 4 outputs/tick, 240 B/output, 16 KiB/flight, 2 ms CPU/tick; 3 overruns → app disabled |
| Faults contained | Panics are caught; the app is disabled and a health message is sent; everything else keeps running |
| Outputs are ground information only | No crew display; results go to maintenance/ops via the queue |
| Apps are data, not code | Declarative TOML rule packages run by the verified rule interpreter in `cfb-core::edge` |
| Configuration control | Each package has a part number + version, an Ed25519 signature, and must be listed in the apps ASCI |

The rule interpreter is part of the airborne software (DAL D, HLR
CFB-HLR-080..083, tested in `edge::tests`). A new or changed *package*
changes only data processed by verified software. Agree this
classification with the DOA: a new app package is expected to be a
**minor change** with a short, standard compliance record (rule review,
test on the simulator, ASCI update), not a re-verification of the box.

## Package format

See `config/apps/turboprop-ops.toml`. Parameter names are the standard
ones (`lat lon gs track alt vs engine airborne nz_max nz_min vib
long_accel`) plus every ARINC parameter from `[[arinc429.labels]]`.

```toml
[app]
id          = "tprop-ops"
part_number = "CFB-APP-0101"
version     = "1.0.0"

[[exceedance]]                 # limit for a time, in a phase
id = "ITT_HIGH"
param = "itt"
above = 800.0
for_s = 5
when = "any"                   # any | airborne | ground
priority = "high"

[[trend]]                      # averaged snapshot when stable
id = "ENG_CRUISE"
params = ["torque", "itt", "ng", "fuel_flow", "oat", "pressure_altitude", "gs"]
stable_s = 120
max_per_flight = 1
stable = [
  { param = "pressure_altitude", band = 100.0, min = 5000.0 },
  { param = "vs", abs_max = 300.0 },
  { param = "torque", band = 3.0 },
]

[summary]
enabled = true
```

Limits in example packages are **examples**. Real limits come from the
AFM, the engine manual and the operator's FDM programme.

## Release and loading

1. Write or change the package; run it on the simulator:
   `cfbd --config <aircraft config> --simulate --apps <dir>`.
2. Review (engineering + whoever owns the limits).
3. Sign: `cfb-release sign release.key tprop-ops.toml` → `tprop-ops.toml.sig`.
4. Add `CFB-APP-0101 1.0.0` to the apps ASCI and sign it.
5. Deliver with an OTA bundle (`/etc/cfb/apps`) or as a separate small
   package into `/data/cfb/apps`. Packages load at start; an unsigned or
   unlisted package is skipped and reported.

## Typical apps

| App | Inputs | Output |
|---|---|---|
| Engine exceedance (ITT, torque, Ng/N1, oil) | ARINC engine labels | start/end, peak, duration |
| Engine trend (ECTM-style) | engine + air data at stable cruise | one snapshot per flight |
| Flight summary / OOOI | phase, engine, GNSS | block/flight time, distance, landings |
| Hard landing / turbulence | own IMU Nz | event with value and position |
| Unstable approach indicators | vs, gs, alt | event |
| Fuel planning feed | fuel flow, gs | consumption per leg |

Engine data use (e.g. for PT6 trend programmes) may be subject to the engine
manufacturer's and the TC holder's data agreements; check before rolling
out.
