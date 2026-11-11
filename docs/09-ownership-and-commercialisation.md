# 09 – Ownership, Protection and Commercialisation

<sub>© 2026 Outthinkx by Luca De Simoni. All rights reserved. Proprietary, not open source. See [LICENSE](../LICENSE).</sub>

> Practical checklist, **not legal advice**. Have a Swiss IP/commercial
> lawyer review the points marked ⚖ before selling or licensing.

## 1. What is in place now

| Item | Status |
|---|---|
| Copyright owner | **Luca De Simoni, trading as Outthinkx**: `LICENSE` (proprietary, all rights reserved, not open source; public visibility for viewing only), notice in every file, `authors` in `Cargo.toml` |
| Licence | None granted. Use only under a written agreement with Outthinkx |
| Not for publication | `publish = false` in Cargo (cannot be pushed to crates.io by mistake) |
| Third-party code | Only permissive crates (MIT / Apache-2.0 / BSD-3 / Unicode / Unlicense): compatible with a closed product. Ship their notices (`NOTICE.md`, `cargo about`) |
| Third-party names / data | Trademark disclaimer and "no proprietary ICD data included" in `NOTICE.md` |

## 2. Immediate actions

1. **Make the GitHub repository private** (planned). Until then the licence
   says "publicly visible for viewing only, not open source", which keeps
   copyright and contract claims intact. Trade-secret protection, however,
   needs secrecy, so do not add new know-how (edge-app rule sets,
   customer data, ICD-derived tables) while the repository is public.
   It is currently **public**.
   While public, anyone can read the code and docs, and the trade-secret
   argument (secrecy) does not hold. GitHub → Settings → General → Danger
   Zone → Change visibility. The licence still forbids use, but secrecy is
   gone for whatever has already been seen or cloned.
2. **Record your human authorship.** Under Swiss law, purely AI-generated
   output is generally not protected by copyright. AI-assisted work can be,
   if your own creative contribution is sufficient; there is no case law
   yet. This code was written with an AI assistant. The provider's terms
   assign the output to you, but protection still depends on your
   contribution. So:
   * keep the design decisions, requirements, reviews and changes as yours
     in the repository history: the concept, architecture choices, module
     concept, certification strategy, review comments and your own
     modifications;
   * keep working on the code yourself; substantial human changes
     strengthen protection;
   * rely in addition on **contracts and trade secrets**, which is what
     Swiss practice recommends anyway. ⚖
3. **Check your employment contract.** ⚖ Under Swiss law:
   * software created in the course of employment belongs to the employer
     (URG Art. 17);
   * inventions made in performing work duties belong to the employer
     (OR Art. 332).

   If you work in aviation, document that this project is done in your own
   time, with your own equipment and outside your duties. Use no employer
   information, documents or data. If unsure, get a written release from
   the employer **before** commercialising.
4. **Use no confidential third-party material** (OEM ICDs, maintenance
   manuals, customer data) in this repository unless you hold a licence that
   allows it. See `08-avionics-data-access.md`.

## 3. Protecting the IP

| Right | What it protects | How / where | Notes |
|---|---|---|---|
| Copyright | Code, docs, designs | Automatic | Proof of date/authorship: signed git tags, a deposit with a notary or a timestamping service |
| Trade secret | Edge-app rule sets, ground analytics, customer data, unpublished designs | Keep secret: private repos, NDAs, access control | Requires secrecy (see 2.1) |
| Trademark "Outthinkx" (and a product name) | Brand | Swiss IPI (IGE), EUIPO for the EU, Madrid system for more countries | Search first for conflicts. Classes likely 9 (hardware/software), 42 (SaaS), 37 (installation) ⚖ |
| Patent | A technical invention, if novel and inventive | IPI / EPO; file **before any public disclosure** | Software as such is excluded, but technical solutions can qualify (e.g. a specific combined link-selection / RF-interlock / update-gate method). Assess novelty with a patent attorney; the public repository may already count as disclosure for what it shows ⚖ |
| Design data ownership in certification | Approved design data (STC / minor change) | Agreement with the DOA | The **approval holder** (DOA) holds the certificate. Keep the design data and IP licensed to them, not transferred ⚖ |

## 4. Business models

| Model | Customer | You sell | Fits |
|---|---|---|---|
| **Ground analytics first** | Operators, CAMOs, flight schools | Edge apps run on existing logs (Garmin CSV, QAR, ACMS files) as SaaS, per aircraft per month | No aircraft change, no certification; revenue and data now (`cfb-replay` is the core) |
| **OEM / TC-holder licence** (primary route) | Aircraft manufacturers with DOA + POA | Design data and software licence, per-unit royalty, support; the OEM's DO certifies, its PO builds or has built (docs/15) | Fastest route into line-fit; they hold the ICDs, the data rights and both approvals |
| **STC partner** | DOA / MRO / avionics shop | Licence to install under their STC (AML across types); per-kit fee | Retrofit market |
| **Box + subscription** | Operators | Hardware kit + monthly service (ground service, OTA, apps, links) | Recurring revenue; needs a production partner and certification |
| **App platform** | Operators, engine OEMs, FDM providers | Signed edge-app packages, marketplace / revenue share | Builds on the signed-package model |

The licence agreements need, at minimum ⚖:
* field of use and territory;
* source code escrow (a certification customer will ask for it);
* ownership of improvements and of data;
* certification responsibility (the DOA's, not yours);
* product liability limits and insurance;
* export-control classification (aviation electronics and encryption).

## 5. Company setup ⚖

* A **GmbH or AG** separates product liability from your private assets.
  Aviation product liability is a real risk.
* Assign the IP from yourself personally into the company with a written
  agreement (or keep it personally and license it to the company).
* Product liability and aviation products insurance before the first
  installation.
* Quality system: a certifiable product will need a supplier quality system
  (ISO 9001 / EN 9100) or a production partner who has one.
