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
| Ground link security | One WireGuard tunnel per bearer to the operator ground service | Outbound only, small, fast handshake over satcom; policy routing by source address pins each tunnel to its bearer |
| Air-ground protocol | CFB frames in UDP, acked by sequence number (`cfbd/transport.rs`, reference ground: `cfb-ground`) | End-to-end delivery, no TCP head-of-line blocking on lossy satcom |
| Edge apps | Declarative signed rule packages (`cfb-core::edge`), see `06-edge-apps.md` | Results instead of raw data on paid links |

## Modules in software

Each hardware module maps to one `cfb-core` component, created only if the
module is in `[modules]`:

| Hardware module | Component | Output |
|---|---|---|
| ARINC 429 RX | `arinc429::Decoder` (configurable label table) | named parameters with SSM + age |
| Reference sensor: GNSS | `gnss::NmeaParser` | own fix, time |
| Reference sensor: IMU | `imu::ImuProcessor`, `imu::EngineVibDetector` | vibration, long. accel, Nz; engine running |
| (all of the above) | `reference::select` | one `Reference`: position, GS, track, altitude, VS, engine, with source and cross-check flags |
| Wi-Fi / Cellular 4G·5G / Satcom | `link::LinkManager` | permitted + chosen bearer |

## Process view

```
 cfb-io  ──(FIFO /run/cfb/inputs, 1 line/s)──▶  cfbd  ──▶ link adapters (per bearer)
   │                                              │
   ├ ARINC 429 RX (SPI): raw words                ├ ARINC decoder, NMEA parser, IMU processor
   ├ own GNSS (UART): raw NMEA                    ├ reference selection + cross-check
   ├ own IMU (SPI/IIO): raw samples, 100 Hz       ├ phase detector
   ├ gpiod (WoW, oil press, maint key, PWR_FAIL)  ├ link policy + RF enables
   ├ ModemManager / iwd / Certus status           ├ reports + IMU advisories → queue
   └ module ID EEPROMs (at boot)                  └ update gate (maintenance page)
```

`cfb-io` only moves raw data; all decoding is in `cfb-core`, so it is
covered by the HLR tests. `cfbd` never touches hardware directly. Everything it decides is in
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
| CFB-HLR-033 | A sent message shall stay queued until acked by the ground, and be resent after `retry_s` | `queue.rs` `peek_ready`/`mark_sent` | `sent_messages_wait_for_ack_then_retry` |
| CFB-HLR-040 | Update only parked, engine off, maint mode, voltage OK, signed, in ASCI, hash OK | `update.rs` | all `update::tests` |
| CFB-HLR-041 | OTA download only over a link permitted for downloads (in flight: configured unmetered links only) | `ota.rs`, `cfbd`, `config.rs` | `happy_path`, `inflight_download_over_cellular_rejected` |
| CFB-HLR-042 | OTA install only with gate open and technician confirmation | `ota.rs` | `happy_path` |
| CFB-HLR-043 | Reboot into the new slot deferred until the gate is open again | `ota.rs` | `reboot_deferred_until_gate_open_again` |
| CFB-HLR-044 | Interrupted download resumes; failed verification discards the package | `ota.rs`, `ota_exec.rs` | `link_loss_aborts_and_resumes`, `bad_package_not_retried_until_new_announcement`, `verify_dir_accepts_signed_package_and_rejects_tampered_bundle` |
| CFB-HLR-050 | Config allowing cellular/Wi-Fi client in flight shall be rejected | `config.rs` `validate` | `cellular_in_flight_is_rejected` |
| CFB-HLR-055 | ARINC 429 words with bad parity, non-normal SSM, wrong channel/SDI or stale age shall not be used | `arinc429.rs` `Decoder` | `decoder_filters_parity_ssm_and_age`, `channel_and_sdi_filters` |
| CFB-HLR-056 | BNR/BCD decoding per ARINC 429 (sign bit 29, up to 20 significant bits) | `arinc429.rs` | `bnr_roundtrip_positive_and_negative`, `known_bnr_vector`, `bcd_decoding` |
| CFB-HLR-057 | NMEA sentences with bad checksum or void fix shall not be used | `gnss.rs` | `rejects_bad_checksum_and_void_fix`, `stale_fix_not_returned` |
| CFB-HLR-058 | IMU samples shall be rotated to aircraft axes per installed mounting | `imu.rs` `Mounting` | `mounting_parse_and_apply` |
| CFB-HLR-059 | Engine running from vibration with on/off delays; no IMU data = unknown | `imu.rs` `EngineVibDetector` | `engine_detection_with_delays` |
| CFB-HLR-060 | Each quantity from the best valid source (aircraft > own sensor; discrete > IMU) | `reference.rs` `select` | `aircraft_source_preferred_when_valid`, `falls_back_to_own_gnss_when_arinc_stale_or_absent`, `engine_discrete_overrides_vibration` |
| CFB-HLR-061 | Aircraft vs own GNSS disagreement shall be flagged and reported | `reference.rs`, `cfbd` | `disagreement_flagged` |
| CFB-HLR-070 | Configured vs detected modules shall be compared and mismatches reported | `modules.rs` | `detection_mismatch_reported` |
| CFB-HLR-071 | A link or input shall only be enabled if its module is fitted | `config.rs` `validate_modules` | `enabled_link_without_module_rejected` |
| CFB-HLR-080 | Edge apps shall only read a data copy and return outputs; no other access | `edge.rs` `EdgeApp`, `DataView` | by construction (interface), reviewed |
| CFB-HLR-081 | Edge outputs limited per tick, per size and per flight | `edge.rs` `EdgeRuntime` | `runtime_enforces_output_and_byte_limits` |
| CFB-HLR-082 | A panicking or CPU-overrunning app shall be disabled without affecting others | `edge.rs` | `panicking_app_is_disabled_others_continue`, `cpu_overrun_disables_after_limit` |
| CFB-HLR-083 | App packages shall be signed and listed in the ASCI | `edge.rs` `load_verified` | `signed_package_must_be_in_asci` |
| CFB-HLR-084 | Exceedance, trend and summary rules per package definition | `edge.rs` `RuleApp` | `exceedance_start_and_end`, `short_exceedance_not_reported`, `trend_needs_stability_and_reports_once_per_flight`, `summary_at_engine_stop` |
| CFB-HLR-090 | Only frames from the configured ground address shall be accepted; link up/down from ground replies (strict) or unanswered sends (metered) | `cfbd/transport.rs` | `send_ack_and_uplink_roundtrip`, `foreign_sender_ignored`, `metered_bearer_optimistic_until_unanswered` |

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

Ground speed comes from the reference selection, so the phase logic keeps
working through a loss of the ARINC source (it falls back to own GNSS).

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
