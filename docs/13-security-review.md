# 13 – Security and Robustness Review

<sub>© 2026 Outthinkx by Luca De Simoni. All rights reserved. Proprietary, not open source. See [LICENSE](../LICENSE).</sub>

Review of the code as of November 2026: the airborne software (`cfb-core`,
`cfb-drivers`, `cfbd`, `cfb-io`), the tools and the OS integration files.
Method: a manual review of every input from outside the process, plus
automated robustness tests (CFB-HLR-110, `crates/cfb-core/tests/robustness.rs`).
The robustness tests feed 20 000 random and mutated inputs per parser and
fail on any panic. Repeat the review for every release; new findings get
the next number.

## 1. Attack surface

| Input | Trust | Reaches | Protection |
|---|---|---|---|
| Ground frames (UDP in WireGuard) | Authenticated tunnel, but treat content as untrusted | `Frame::decode`, ack parser | Only the configured ground address is accepted; CRC; length limits |
| SBD gateway messages (ground side) | Untrusted (HTTP webhook) | `sbd::from_hex`, `unpack`, `Frame::decode` | Robustness-tested; webhook token (`cfb-ground`) |
| GNSS serial stream | Untrusted bytes | `NmeaParser` | ASCII only, checksum, robustness-tested |
| ARINC 429 words | Aircraft data, may be faulty | `Decoder`, `Survey` | Parity, SSM, age; robustness-tested |
| Modem responses (SBD) | Device, may misbehave | `link_sbd::Modem` | Timeouts, bounded buffer |
| `cfb-io` → `cfbd` pipe | Local, own process | `input::parse_line` | Bounded line length, lossy UTF-8 |
| OTA packages | Untrusted until verified | `UpdateGate::verify`, RAUC | Ed25519 (manifest + ASCI), hash, size, ASCI listing, RAUC X.509 |
| Edge app packages | Untrusted until verified | `AppPackage::load_verified` | Ed25519 + ASCI; sandboxed runtime |
| Config file | Signed, on the read-only rootfs | `Config::from_toml` | Validation rejects unsafe combinations; robustness-tested |
| Maintenance page | Local network, people | `cfb-maint` | Bound to the maintenance address, token, separate process with no radio/update rights |

## 2. Findings

| # | Finding | Severity | Status |
|---|---|---|---|
| R1 | `NmeaParser` sliced strings at byte offsets: a GNSS sentence with a non-ASCII byte panicked → `cfbd` crash and restart loop on a noisy GNSS line | High (availability) | **Fixed:** ASCII check + non-panicking slicing; robustness test |
| R2 | `sbd::from_hex` same slicing bug, reachable from the ground webhook | Medium | **Fixed:** ASCII check; robustness test |
| R3 | `nmea_serial::is_wanted` same pattern in the driver | High (availability) | **Fixed** + unit test |
| S1 | OTA download built a shell script containing the update URL in single quotes. A quote in the URL (config is signed, but still) would allow command injection | Medium | **Fixed:** `curl` runs directly per file, arguments not interpreted; test with a quote in the path |
| S2 | `cfbd` read input lines without a length limit; invalid UTF-8 stopped input processing | Medium (availability) | **Fixed:** 256 KiB limit, bounded discard, lossy decoding; test |
| S3 | SBD modem response buffer could grow without limit on junk without line ends | Low | **Fixed:** 4 KiB cap |

## 3. Accepted by design (with reasoning)

| Item | Reasoning |
|---|---|
| polkit lets user `cfb` reboot, power off, install RAUC bundles and switch radios | Needed for OTA, power-fail and RF inhibit. A compromised `cfbd` could reboot or switch radios **off**; it can't install unsigned software (RAUC checks signatures). Radios **on** in flight is prevented by the hardware interlock on the installed unit |
| Verify-then-install gap: the bundle in `/data/updates` could be swapped after verification by someone with `cfb` rights | RAUC verifies the bundle signature itself at install time |
| Edge app CPU budget is measured after the call | Apps are declarative rules without loops; the interpreter is verified; panics are caught |
| Ground UDP service accepts any peer inside the tunnel | WireGuard authenticates peers; the box identity is the tunnel address |
| Development signing key in example configs | Example only; release requires your own key on a hardware token (`docs/05`) |

## 4. Open items

1. Set the system time from GNSS before the first frame of a flight (frame
   time stamps today come from the system clock / RTC).
2. Secure boot and dm-verity in the OS image (`os/yocto`, item for the RAUC
   image).
3. Rate limiting on the ground HTTP endpoints (put a reverse proxy in front in
   production).
4. Periodic dependency audit (`cargo audit` in CI once network access is
   available to the CI job).
