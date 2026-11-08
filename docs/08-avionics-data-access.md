# 08 – Getting Data out of Honeywell, Garmin and Other Avionics

> Research summary, November 2026. Public sources only; items marked
> *unconfirmed* need checking against the manufacturer's current documents.
> Copyright © 2026 Outthinkx by Luca De Simoni. Proprietary.

## 1. The data paths that exist on an aircraft

| # | Path | What you get | Format / standard | Who controls the decoding |
|---|---|---|---|---|
| A | **ARINC 429 buses** between LRUs | Live nav, air data, engine, status | ARINC 429; labels per ARINC 429 Part 1 and the **GAMA ARINC 429 General Aviation Subset (Publication No. 11, v6.0)** | Standard labels: SAE/ARINC and GAMA (purchasable). OEM-specific labels: the avionics/TC holder's ICD |
| B | **Maintenance / data-loader port** (Ethernet) | Fault history, trend/condition-monitoring files, database loading | ARINC 615A (Ethernet data loading) or OEM protocol | Avionics OEM |
| C | **Central maintenance computer (CMC) / ACMS downloads** | Fault history DB (FHDB), ACMS/condition-monitoring files, exceedances | OEM file formats | Avionics OEM, TC holder |
| D | **FDR / QAR stream** | Everything the flight data recorder records | ARINC 717 frames; the parameter layout is documented in **FRED (ARINC 647A, XML)** | Operator/OEM owns the dataframe documentation |
| E | **Avionics' own data log** (SD card) | 1 Hz flight + engine data | Garmin G1000-family: CSV on the MFD SD card, documented in the pilot's guides | Readable by the operator |
| F | **Engine data units** | Engine trend, exceedances | e.g. P&WC FAST, PT6E digital engine data | Engine manufacturer |

## 2. Honeywell Primus Apex (PC-12 NG / NGX)

* **Maintenance plug:** the Pilatus data loading guide for Primus Apex
  (PC-12/47E) describes a **maintenance panel with two RJ45 Ethernet
  connectors, NIC1 and NIC2**. They are used with a laptop for database
  loading and for downloading **fault history and trend data**. Loading is
  also possible from SD cards. *(Applicability to the NGX revision:
  unconfirmed.)*
* **Central maintenance computer:** Apex and Epic have an integrated CMC
  that records system and engine operating and fault data.
* **Wireless path already certified:** the **Aspen CG100P connected
  gateway**, developed with Pilatus, Honeywell and Jeppesen, is covered by
  the PC-12 NG type certificate. It does wireless database loading,
  maintenance downloads and flight-plan transfer. Honeywell's MyCMC app
  retrieves ACMS and FHDB files from Apex through it. According to
  Honeywell, ForeFlight flight-plan transfer via the gateway works on the
  PC-12 NG, NGX and PC-24.
* **Engine:** the PT6E-67XP (NGX) is "digitally connected" with 100+
  monitored parameters, analysed by P&WC. P&WC FAST was extended to the
  PC-12NG (2018).

## 3. Honeywell Primus Epic / Pilatus ACE (PC-24)

* ACE is based on Primus Epic. Honeywell offered a wireless data loader for
  Epic cockpits (2015) that downloads maintenance data.
* Pilatus' free iPad app **PC-24 Technical Data Analyzer** retrieves and
  analyses ACM (aircraft condition monitoring) files via the Aspen
  gateway: event and exceedance data.
* **Line-fit connectivity: Astronautics AeroSync Max** (2025). It
  automatically downloads flight and maintenance data after every flight
  over cellular or Wi-Fi to Pilatus' CrystalCare servers.

## 4. Others

* **Garmin G1000 / G950 / G1000 NXi** (and similar): CSV data log on the
  MFD's SD card, 1 record per second, file per power cycle. The pilot's
  guides document the columns. The parameter set depends on the
  installation (typical: position, altitudes, speeds, attitude,
  accelerations, electrical, fuel, engine). → `cfb-core::datalog` +
  `cfb-replay` read it today. Public sources do not describe the G3000
  column set: *unconfirmed*.
* **Garmin 429 outputs**: the label sets are in the installation manuals
  (e.g. GNS/GTN family), available to installers.
* **Quick access recorders (QAR)**, e.g. **Avionica miniQAR / miniAID**
  (FAA-PMA parts; FAA AML STC for miniQAR plus the avCM 4G module on Part 23
  turboprops and jets). They record the ARINC 717 stream next to the FDR
  and offload it by cellular. Decoding needs the aircraft's dataframe
  (FRED).
  *The unit in the photo (Avionica, FAA-PMA, eligibility doc 8040001ED) is
  of this family; confirm the model from its part-number label.*
* Acron µQAR, L3Harris mQAR: other QARs with CF card / USB offload.
* **Engine monitors** on piston aircraft (JPI, EI, Garmin EIS): serial
  outputs, formats documented to installers.

## 5. What this means for the CFB

**1. Two very different kinds of port.** Paths A, D and E are
*receive-only* data. Path B, the maintenance network port, is a
*two-way network* into the avionics. Plugging the CFB into NIC1/NIC2
would destroy the "cannot affect the avionics" argument that keeps the CFB
a minor change with level D software. It would also mean speaking
Honeywell's maintenance protocol, which needs Honeywell's licence and
specification.
→ **Do not connect to the maintenance network** except through an
OEM-approved gateway and protocol, on ground only, under a Honeywell
agreement.

**2. Use what is already approved.** On the PC-12 and PC-24, Aspen's
gateway, AeroSync Max, FAST and the QARs already take data off the
aircraft. The fastest route to market is to **integrate on the ground
side**. Take the files these systems already deliver (ACMS, FHDB, QAR,
Garmin CSV) and run the CFB edge apps and analytics on them (`cfb-replay`
is the start). That needs no aircraft change.

**3. For the in-flight box**, ARINC 429 receive-only (path A) remains the
clean route. Use GAMA/ARINC standard labels where the avionics outputs
them, and the TC holder's or OEM's ICD for the rest.

**4. Decoding rule.** Use only data definitions you have a right to use:
* published standards (ARINC 429 Part 1, GAMA Publication 11, ARINC
  717/647A), bought from SAE/ARINC/GAMA;
* manufacturer documentation provided to you under licence (ICDs,
  installation manuals, FRED dataframes from the operator or OEM);
* formats documented to operators (Garmin data log).

The bench tool `cfb-a429-survey` only **checks** a label table against a
real bus. It does not guess meanings.

## 6. Legal notes on decoding without documentation (not legal advice)

* **Switzerland:** URG Art. 21 allows decoding a program only to get
  interface information needed for interoperability of an independently
  developed program, when that information is not otherwise accessible. The
  Copyright Ordinance (URV Art. 17) says the rights holder is unreasonably
  prejudiced in particular when the information is used to make a
  **competing product**.
* **EU:** under the Trade Secrets Directive (2016/943), reverse
  engineering a lawfully acquired product is a lawful way to obtain
  information **unless contractually excluded** (Recital 16, Art. 3).
  Using the information can still be unlawful if a contract restricts it
  (Art. 4).
* Avionics manuals, ICDs and maintenance software come with licence terms
  that usually prohibit exactly this. Aircraft operators are often bound
  by OEM support agreements (e.g. CrystalCare).
* Certification also needs the data definitions as **approved design
  data**. Reverse-engineered label meanings would not be accepted as a
  basis for an approval.

**Conclusion:** get the ICDs and data rights by agreement (TC holder,
Honeywell, engine OEM, or the operator for FRED dataframes). Reverse
engineering is legally risky in exactly the commercial scenario you are
planning, and it is useless for certification anyway.

## 7. Competitive landscape (PC-12 / PC-24)

| Product | Owner | What | Status |
|---|---|---|---|
| AeroSync Max | Astronautics | Post-flight flight and maintenance data download (cellular/Wi-Fi) to Pilatus CrystalCare | Line-fit PC-24 (2025) |
| CG100P Connected Gateway | Aspen Avionics | Wireless data loading + maintenance download for Apex; ForeFlight link | On the PC-12 NG TC; NG/NGX/PC-24 per Honeywell |
| FAST | P&WC | Engine and aircraft data acquisition + wireless transmission | Extended to PC-12NG (2018) |
| miniQAR + avCM / miniAID | Avionica | QAR + 4G offload, FAA AML STC Part 23 | Retrofit |

**CFB positioning:**
* *in flight, live* (Starlink/LEO, Iridium), which the post-flight
  systems above don't do;
* *edge apps on board* (results instead of raw data);
* *multi-source* (ARINC + own GNSS/IMU + logs from the existing systems);
* *one box and one software across types and avionics brands*;
* *OTA-managed app packages*.

Partnering, for example as the in-flight and edge layer next to the
existing post-flight systems, is likely easier than replacing them.

## Sources

* [Pilatus data loading guide (Primus Apex), NIC1/NIC2 maintenance panel](https://www.scribd.com/document/817800852/Pilatus-Data-loading-guide)
* [Honeywell: Integrative initiatives – Apex/Epic CMC, MyCMC](https://www.aviationpros.com/engines-components/aircraft-airframe-accessories/article/12040188/integrative-initiatives-how-honeywell-is-streamlining-aircraft-system-maintenance)
* [AIN: iPad apps are key Honeywell avionics functionality (CG100P)](https://backend.ainonline.com/aviation-news/business-aviation/2013-07-03/ipad-apps-are-key-honeywell-avionics-functionality)
* [Aspen: Connected Panel on Pilatus PC-12 NG](https://aspenavionics.com/news/aspens-connected-panel-technology-takes-flight-on-pilatus-pc-12-ng-aircraft)
* [Honeywell: additional ForeFlight integration on Pilatus aircraft](https://www.honeywellaerospace.com/us/en/insights/articles/additional-foreflight-integration-pilatus-aircraft)
* [AIN: wireless data loader for Primus Epic cockpits](https://www.ainonline.com/aviation-news/business-aviation/2015-11-24/wireless-data-loader-offered-primus-epic-cockpits)
* [Astronautics: AeroSync Max line-fit on PC-24](https://www.astronautics.com/astronautics-aerosync-max-now-line-fit-connectivity-system-on-pilatus-pc-24/)
* [AIN: PC-24 adopts AeroSync Max](https://backend.ainonline.com/aviation-news/business-aviation/2025-10-09/pc-24-adopts-aerosync-max-flight-data-connectivity)
* [App Store: PC-24 Technical Data Analyzer](https://apps.apple.com/us/app/pc-24-technical-data-analyzer/id1441361981)
* [AIN: P&WC focuses on prognostication (FAST on PC-12NG)](https://ainonline.com/aviation-news/business-aviation/2018-05-25/pwc-focuses-prognostication)
* [RTX: PT6E-67XP digitally connected engine](https://www.rtx.com/en/prattwhitney/blogs/airtime/2020/10/05/pt6-e-series-engine-drives-step-change)
* [Pilatus CrystalCare](https://pilatus-aircraft.com/customer-support/crystalcare)
* [SAE: ARINC 615A-4 Ethernet data loader](https://saemobilus.sae.org/standards/arinc615a-4-615a-4-software-data-loader-using-ethernet-interface)
* [GAMA technical publications (ARINC 429 GA Subset, Pub. 11)](https://gama.aero/facts-and-statistics/publications/gama-and-industry-technical-publications-and-specifications)
* [SAE: ARINC 429 Part 1-19 label assignments](https://saemobilus.sae.org/standards/arinc429p1-19-429p1-19-digital-information-transfer-system-dits-part-1-functional-description-electrical-interfaces-label-assignments-word-formats)
* [Wikipedia: ARINC 717](https://en.wikipedia.org/wiki/ARINC_717)
* [SAE: ARINC 647A-1 FRED](https://saemobilus.sae.org/standards/arinc647a-1-647a-1-flight-recorder-electronic-documentation-fred)
* [G1000 data logging (pilot's guide extract)](https://www.manualslib.com/manual/1396612/Garmin-G1000.html?page=634)
* [G1000 data logging, user description](https://blahg.josefsipek.net/?p=471)
* [Avionica: AML STC for miniQAR/avCM on light jets](https://www.aviationpros.com/engines-components/aircraft-airframe-accessories/avionics/press-release/12407825/avionica-inc-avionica-earns-aml-stc-for-miniqar-avcm-on-light-jets)
* [Avionica miniAID announcement](https://runwaygirlnetwork.com/2021/10/12/press-release-avionica-announces-new-miniaid-product)
* [Swiss Copyright Act Art. 21](https://lawbrary.ch/law/art/URG-v2022.01-en-art-21) · [Copyright Ordinance Art. 17](https://droit-bilingue.ch/en-de/2/23/231.11-17-27.html)
* [EU Trade Secrets Directive 2016/943 (WIPO Lex)](https://www.wipo.int/wipolex/en/legislation/details/16435) · [Pinsent Masons analysis](https://www.pinsentmasons.com/out-law/analysis/courts-will-shape-the-extent-trade-secrets-can-be-protected-says-expert-)
