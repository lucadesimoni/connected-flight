# Software Configuration Management Plan (SCMP)

<sub>© 2026 Outthinkx by Luca De Simoni. All rights reserved. Proprietary, not open source. See [LICENSE](../../LICENSE).</sub>

> **DRAFT** for review with the design organisation (DOA). Items in
> ⟨angle brackets⟩ are decisions or data of the DOA / applicant. Not an
> approved document.

## 1. Identification

| Item | Identification |
|---|---|
| Software part | `CFB-SW-nnnn` + SemVer version; one part number per released image |
| Hardware part | `CFB-HW-nnn` + revision; module ID EEPROMs |
| Edge app package | `CFB-APP-nnnn` + version |
| Configuration file | per aircraft installation, signed, listed in the installation records |
| Source baseline | git tag `vX.Y.Z` on a release branch |

## 2. Baselines and branches

`main`: development, always green. `release/x.y`: one per certified
baseline, only fixes; every fix also on `main`. Tags are signed.

## 3. Change control

Problem reports / change requests ⟨tracker⟩ with: description,
classification (minor/major per DOA procedure), affected requirements,
verification, release. No change without a record.

## 4. Configuration index and archive

`scripts/sci.py` per release: commit, toolchain, all dependencies with
checksums (`Cargo.lock`), hashes of binaries and configuration/OS files.
Archive per release: source tarball, `Cargo.lock`, vendored dependencies
(`cargo vendor`), toolchain installer, SCI, trace matrix, test logs, built
images and bundles, for the life of the installations ⟨retention period⟩.

## 5. Load control

Signed bundles and manifests, ASCI per hardware part, update gate, install
records (`docs/05`). Release keys on a hardware token ⟨custodian⟩.
