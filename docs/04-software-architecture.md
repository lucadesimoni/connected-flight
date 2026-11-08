# 04 – Software Architecture

## Stack

| Layer | Choice | Why |
|---|---|---|
| OS image | Yocto (meta-raspberrypi + meta-rauc) or Buildroot, minimal | Reproducible builds, small attack surface, known package list for the SCI |
| Root filesystem | read-only ext4/squashfs + dm-verity, A/B slots | Power-cut safe, tamper evident, enables rollback |
| Data partition | `/data`, ext4, `commit=60`, `noatime` | Queue snapshots and logs only; limits flash wear |
| Updates | RAUC with signed bundles, boot counter | Atomic A/B install, automatic fallback |
| Supervision | systemd, `WatchdogSec`, external HW watchdog | Restart on hang, reset on deadlock |
| Application | Rust, `#![forbid(unsafe_code)]`, no async runtime | Memory safety, deterministic 1 Hz loop, ~600 KB binary |
| I/O adapters | `cfb-io` (separate service): gpsd, libgpiod, ModemManager, iwd, Certus AT driver | Keeps drivers out of the decision logic; writes one input line/s |
| Ground link security | WireGuard tunnel to operator ground service | Outbound only, small, fast handshake over satcom |

## Process view

```
 cfb-io  ──(FIFO /run/cfb/inputs, 1 line/s)──▶  cfbd  ──▶ link adapters (per bearer)
   │                                              │
   ├ gpsd (GNSS)                                  ├ phase detector
   ├ gpiod (WoW, oil press, maint key, PWR_FAIL)  ├ link policy + RF enables
   ├ ModemManager / iwd / Certus status           ├ store-and-forward queue
   └ ARINC/serial/CAN parsers → messages          ├ report generator
                                                  └ update gate (maintenance page)
```

`cfbd` never touches hardware directly. Everything it decides is in
`cfb-core`, which has no I/O apart from the queue snapshot. That is what makes
the HLR tests meaningful: the tests exercise the same code that flies.

## High-level requirements (HLR)

| ID | Requirement | Implemented | Verified by |
|---|---|---|---|
| CFB-HLR-010 | Unknown phase shall be treated as airborne | `phase.rs` `is_confirmed_ground` | `starts_unknown_without_inputs`, link tests |
| CFB-HLR-011 | WoW shall be the primary phase source | `phase.rs` `indicate` | `wow_ground_needs_confirm_time` |
| CFB-HLR-012 | WoW=ground with GS > airborne threshold shall give Unknown immediately | `phase.rs` | `wow_disagreeing_with_gnss_goes_unknown_immediately` |
| CFB-HLR-013 | Without WoW, GS with hysteresis band shall decide | `phase.rs` | `takeoff_and_landing_with_gnss_only`, `short_gust_does_not_flip_phase` |
| CFB-HLR-014 | Input loss > timeout shall give Unknown | `phase.rs` | `input_loss_times_out_to_unknown` |
| CFB-HLR-020 | Ground-only links shall transmit only with ground confirmed | `link.rs` `tx_permitted` | `cellular_and_wifi_never_transmit_unless_ground_confirmed` |
| CFB-HLR-021 | Links shall carry only priorities ≥ their minimum | `link.rs` `select` | `airborne_uses_satcom_for_critical_only` |
| CFB-HLR-022 | Metered links shall respect a per-flight byte budget | `link.rs` | `flight_budget_caps_satcom` |
| CFB-HLR-023 | Cheapest permitted link shall be chosen | `link.rs` | `ground_prefers_cheapest`, `leo_preferred_over_iridium_in_flight_when_enabled` |
| CFB-HLR-030 | Queue shall be byte-bounded; evict lowest priority oldest first, never higher priority | `queue.rs` `push` | `eviction_lowest_priority_oldest_first`, `low_priority_never_evicts_higher` |
| CFB-HLR-031 | Expired messages shall be dropped | `queue.rs` `expire` | `ttl_expiry` |
| CFB-HLR-032 | Corrupt/missing snapshot shall give empty queue, not a failure | `queue.rs` `load_or_new` | `save_and_load_file`, `snapshot_roundtrip_and_corruption` |
| CFB-HLR-040 | Update only parked, engine off, maint mode, voltage OK, signed, in ASCI, hash OK | `update.rs` | all `update::tests` |
| CFB-HLR-050 | Config allowing cellular/Wi-Fi client in flight shall be rejected | `config.rs` `validate` | `cellular_in_flight_is_rejected` |

## Phase logic

```
             WoW=air or GS ≥ 50 kt for 5 s
  ┌────────┐ ───────────────────────────────▶ ┌──────────┐
  │OnGround│                                  │ Airborne │
  └────────┘ ◀─────────────────────────────── └──────────┘
      ▲      WoW=ground or GS ≤ 35 kt for 20 s     │
      │                                            │
      │   ground confirmed     inputs lost / WoW=ground but fast
      │                            (immediate)     ▼
      └──────────────────────────────────── ┌─────────┐
                                            │ Unknown │  = treated as airborne
                                            └─────────┘
```

Takeoff and landing rolls (WoW on, speed high) show up as `Unknown`. That is
on purpose: ground-only radios are already off during the roll.

## Data efficiency (in flight)

| Item | JSON over HTTPS | CFB frame |
|---|---|---|
| Position report | ~150 B payload + TLS/HTTP overhead (~1 KB+) | **30 B** incl. header and CRC |
| Per hour at 1/min | ~60 KB+ | **1.8 KB** |
| Iridium budget default | – | 64 KiB/flight, ≈ 2000 reports |

Further savings available without format changes: batch several reports in
one satcom session; delta-encode positions in a batch.

## Resource targets

| | Target | Measured (x86_64 dev build) |
|---|---|---|
| `cfbd` binary | < 1 MB | 0.6 MB release, stripped |
| `cfbd` RSS | < 16 MB | to measure on target |
| CPU | < 5 % of one core | to measure on target |
| Flash writes | ≤ 1 snapshot/min | by design |
