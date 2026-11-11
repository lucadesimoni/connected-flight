# 17 – Load Test: Bus Data, Edge Computing and Bursts

<sub>© 2026 Outthinkx by Luca De Simoni. All rights reserved. Proprietary, not open source. See [LICENSE](../LICENSE).</sub>

> Development evidence, **not certification credit**. The figures were
> measured on a development container (Intel Xeon 2.3 GHz, `cfbd` pinned to
> one core). Repeat the run on the target (Pi 5 / CM5) with
> `scripts/loadtest.sh` before quoting numbers to an OEM. A Cortex-A76 at
> 2.4 GHz is expected in the same range, within a factor of 2–3.

## 1. What is tested

| Load | Value | Why this value |
|---|---|---|
| ARINC 429 | **11,000 words/s**: 4 channels, 64 labels, high speed (100 kbit/s) | Close to the physical maximum of 4 busy high-speed buses; real CS-23 buses carry far less |
| IMU | 100 Hz, 3 axes | Rate used by the portable kit |
| GNSS | RMC + GGA at 1 Hz | Normal NMEA output |
| Edge | **128 exceedance rules**, 16 trend rules, 1 summary in one signed app | Far above a typical rule package |
| Storm burst | All 128 parameters exceed their limit at the same time for 30 s | Worst case for the edge output path |
| Flood burst | 60 s with ARINC input lines over the 256 KiB limit | Malformed or runaway adapter |
| Link outage | Ground station killed for 4 s real (≈ 200 simulated s) during a storm | Queue, retry, dedup and drain |

Tools:
* `cfb-loadgen setup` writes a config, a signed load app and a signed ASCI
  (ephemeral test key);
* `cfb-loadgen run` writes the input lines that `cfb-io` and an ARINC adapter
  would write, as fast as possible or paced;
* `cfbd` reports the tick-work histogram (mean, p50, p99, max), peak memory
  (VmHWM), truncated input lines and the edge backlog at exit; the status
  file has `tick_work_us_last`, `tick_work_us_max`, `input_truncated` and
  `edge_backlog`.

## 2. Results

### Scenario A + B: one simulated hour, all loads, storm at t=1200 s, flood at t=2400 s

| Measure | Before the fixes | After the fixes |
|---|---|---|
| ARINC words processed | 40.3 M | 40,318,237 |
| Wall time for 3600 simulated s | 8.1 s | 8.0–8.2 s (**≈ 450× real time**) |
| Tick work mean / p99 / max | 1078 / 2500 / 6025 µs | 1062 / 1900 / 6436 µs |
| Share of the 1 s tick budget (p99) | 0.25 % | **0.19 %** |
| Peak memory (VmHWM) | 4.8 MB | **4.8 MB** |
| Edge outputs from the storm | 4 START, 4 END (124 of 128 dropped) | **128 START, 128 END**, 24 TREND |
| ARINC parity errors / unknown labels | 35 / 25 (from cut lines) | **0 / 0** |
| GNSS fault during the flood | yes (false) | **no** |
| Health messages for the flood | one per line | 2 (rate limited, 60 lines counted) |

Normal operation without bursts: tick mean 4 µs, p99 18 µs, 3.9 MB.

### Scenario C: end to end over UDP to `cfb-ground` (SQLite), outage during a storm

900 simulated s at 50× speed, ground-only links, storm at t=250 s for 30 s,
ground station down from ≈ t=200 s to ≈ t=375 s.

| Measure | Result |
|---|---|
| Exceedance START / END in the ground database | **128 / 128** |
| Positions in the ground database | 15 |
| Duplicates in the ground database | **0** |
| Queue peak | 259 messages at t=364 |
| Edge backlog peak | 140 outputs |
| Queue empty again | t=398, **23 s after the link was back** |
| Still queued at the end | 0 |
| Tick work mean / p99 / max | 1203 / 2000 / 2289 µs |
| Peak memory | 4.2 MB |

## 3. Findings and fixes

| # | Finding | Effect | Fix | Requirement / test |
|---|---|---|---|---|
| 1 | An input line over 256 KiB was cut at the limit. Tokens after `a429=` (GNSS) were lost, and the cut 8-digit word was parsed as a short word | False GNSS fault, parity errors and unknown labels | Fixed token order: bulk data last (`imu`, then `a429`); `cfb-io` and `cfb-loadgen` follow it; `a429` words accepted only with exactly 8 hex digits; truncation counted and reported | CFB-HLR-111, `truncated_line_keeps_early_tokens_and_drops_partial_words` |
| 2 | The edge runtime dropped outputs above `max_outputs_per_tick` | 124 of 128 storm events lost | Bounded per-app backlog (`max_backlog`, default 256): outputs queue and go out over the next ticks; only the excess over the backlog is dropped; older configs load unchanged | CFB-HLR-085, `burst_is_delivered_completely_over_following_ticks`, `backlog_overflow_drops_only_the_excess`, `limits_from_older_config_without_new_field` |
| 3 | One health event per dropped output or per cut line | Health channel flooded during a burst | Drops aggregated per app and reason, input truncation reported at most once per 60 s | CFB-HLR-085, CFB-HLR-111 |

## 4. Headroom and conclusions

* CPU: the worst case uses under 0.2 % of the tick budget at p99 and under
  0.7 % at max on one core. The box has margin for far more edge rules, and
  the other cores stay free for links, OTA and the maintenance page.
* Memory: under 5 MB resident in every scenario, against 4–8 GB on the
  target. Queues and backlogs are bounded, so memory stays flat over time.
* Bursts: no event is lost in a 128-parameter storm, also with the ground
  link down; the queue drains about 10 messages per simulated second once
  the link is back.
* Not covered here: real ARINC hardware timing (adapter FIFO, kernel
  scheduling), thermal throttling at 70 °C, real cellular/Iridium latency.
  These belong to the hardware test (docs/11 and `scripts/verify-hardware.sh`).

## 5. How to rerun

On the target, after `cargo build --release`:

```sh
scripts/loadtest.sh /var/tmp/cfb-load
```

It runs scenarios A+B and C, prints the figures and checks the pass
criteria (p99 < 100 ms, max < 500 ms, 128/128 storm events locally and on
the ground, no parity errors, no false GNSS fault, empty queue at the end).
Exit status 0 means all criteria passed.

Single runs:

```sh
cfb-loadgen setup --dir D --labels 64 --channels 4 --rules 128 [--transport host:port]
cfb-loadgen run --config D/loadtest.toml --seconds 3600 --words-per-sec 11000 \
    --imu-hz 100 --storm 1200:30 --flood 2400:60 | taskset -c 1 cfbd --config D/loadtest.toml
```
