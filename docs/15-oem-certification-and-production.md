# 15 – OEM Route: Certification by the OEM's DO, Production by its PO

<sub>© 2026 Outthinkx by Luca De Simoni. All rights reserved. Proprietary, not open source. See [LICENSE](../LICENSE).</sub>

Aircraft OEMs such as Pilatus hold both a **Design Organisation Approval
(DOA, Part 21 Subpart J)** and a **Production Organisation Approval (POA,
Part 21 Subpart G)**. The expected route for the CFB:

* the OEM's **DO** certifies the installation as a change to its own type
  design (or as an option on its type certificate), under its own
  procedures and privileges;
* the OEM's **PO** builds the box itself, or has it built by a supplier
  under its supplier control, and releases each unit (EASA Form 1 /
  production release per its POA);
* **Outthinkx** delivers and licenses the design: technical data package,
  software with its life-cycle data, and engineering support.

This is the shortest route to a certified, line-fitted product. The OEM
has the type design data, the avionics ICDs, the data rights and the
approvals. It also decides the shape of everything Outthinkx delivers,
so the repository is organised to be handed over.

## 1. What the OEM's organisations need, and where it is

| OEM function | Needs | Provided by (repository / to be produced) |
|---|---|---|
| DO – systems | Functional description, interfaces, safety assessment input | docs/01, docs/03, [docs/16 ICD](16-interface-control.md), [docs/cert/FHA](cert/FHA.md) |
| DO – software | Software life-cycle data to ED-12C / DO-178C Level D | requirements (docs/04), code, tests, trace matrix + SCI (CI evidence), plan drafts (docs/cert) to merge into the OEM's own procedures |
| DO – equipment / hardware | COTS argument (AMC 20-152A), qualification evidence | docs/11 §B, docs/14 §4 (same chips from kit to unit), DO-160G test plan and report (to be produced on the production unit) |
| DO – installation | Installation drawings, electrical load, wiring, antennas, ICA | ICD (docs/16); drawings by the OEM's installation engineering with Outthinkx support |
| DO – security | Information security assessment input | docs/13, docs/02 §6 |
| PO – production | Production data: BOM with Approved Vendor List, drawings, assembly and test instructions, acceptance test procedure (ATP), programming and configuration procedure | to be produced with the carrier board (rev B onwards); ATP starts from `cfb-verify-hardware` |
| PO – configuration | Part numbers, software loading, configuration control | SCMP draft, ASCI, signed bundles (docs/05) |
| Continued airworthiness | ICA, AFMS input, service bulletins, OTA release process | docs/05, docs/10 (procedures), to be produced per type |

The plan drafts in `docs/cert` are **inputs** for the OEM's DO, written so
they can be absorbed into the OEM's own procedures and handbook. They
don't replace them.

## 2. Built to be integrated

| Property | How |
|---|---|
| One software for all builds and types | Hardware modules and aircraft specifics in signed configuration (docs/12) |
| OEM data stays OEM data | ARINC label tables, limits and edge rules are configuration files from the OEM's ICDs; nothing proprietary is in the code |
| Safety by hardware | Receive-only interfaces, hardware RF interlock: software changes stay minor (FHA) |
| Clear interfaces | ICD (docs/16): electrical, data and software interfaces, versioned |
| Evidence on every build | Trace matrix and configuration index from CI |
| Production test built in | `cfb-verify-hardware` is the seed of the production ATP; status page and status snapshot for end-of-line checks |
| Long-term support | Pinned toolchain, LTS OS base, release branches, compatibility rules (docs/12) |
| Supply chain | Industrial / automotive parts, second sources, chips that carry from kit to unit (docs/14 §4) |

## 3. Production options (decided with the OEM)

| Option | Who builds | Outthinkx role | Fit |
|---|---|---|---|
| A. OEM builds in-house | OEM PO | design authority support, technical data package, software releases | Simplest approval; OEM controls everything |
| B. Contract manufacturer under the OEM's POA supplier control | EMS supplier (EN 9100) | as A, plus supplier technical support | Typical for electronics; OEM audits the supplier |
| C. Outthinkx as approved supplier | Outthinkx (or its EMS partner) under the OEM's supplier control | supplies finished units | Needs a quality system (EN 9100) on Outthinkx's side |

Options A or B are realistic first; C becomes possible once Outthinkx has
a certified quality system.

## 4. Commercial and IP structure (with docs/09)

* **Licence, not transfer:** the OEM receives a licence to use the design
  data and software for its types; Outthinkx keeps the IP and the right to
  license other OEMs (field-of-use and exclusivity terms negotiable).
* **Design data for the approval:** the OEM's DO needs to hold or control
  the approved design data. Typical solution: a data licence for the
  approval, plus **source-code escrow**, so the OEM can keep the type design
  supportable if Outthinkx can't.
* **Royalty per unit / per aircraft** plus engineering support and
  software maintenance fees; optional revenue share on edge apps and
  ground services.
* **Responsibilities:** certification and production release lie with the
  OEM's approved organisations; Outthinkx warrants the delivered design data
  as specified and supports the OEM's investigations (occurrence reports,
  airworthiness directives).

All of this needs a Swiss / EU aviation-contract lawyer ⚖.

## 5. Next steps toward an OEM

1. A portable-kit demonstrator with real data from flights (docs/10, docs/14).
2. A carrier board rev A demonstrating the safety-by-hardware features
   (interlock, receive-only, power stage) plus an early EMI pre-scan.
3. A technical presentation to the OEM's DO: architecture, FHA, software
   evidence, ICD.
4. NDA and data agreement → OEM ICDs → bench integration on the OEM's
   avionics test bench.
5. Development agreement: who produces which certification data, under
   which DO procedures, the production option (§3), licence and escrow.
