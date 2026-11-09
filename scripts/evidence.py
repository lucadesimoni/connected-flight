#!/usr/bin/env python3
# Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
# Proprietary software, not open source. See LICENSE.
"""Requirements-to-test trace matrix (certification evidence).

Reads the HLR table in docs/04-software-architecture.md, runs the whole test
suite, and writes target/evidence/trace-matrix.{md,csv}.

"Verified by" column conventions:
  `test_name`        a test function (matched on the last path segment)
  `module::tests`    every test under that module path
  `tests/file.rs`    every test of that integration-test file
  anything else      review / bench evidence (listed, not executed here)

Exit status 1 if a referenced test is missing or failed.
"""
import csv, os, re, subprocess, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "target", "evidence")


def hlr_rows():
    rows = []
    for line in open(os.path.join(ROOT, "docs", "04-software-architecture.md"), encoding="utf-8"):
        if not line.startswith("| CFB-HLR-"):
            continue
        cols = [c.strip() for c in line.strip().strip("|").split("|")]
        if len(cols) < 4:
            continue
        rid, req, impl, ver = cols[0], cols[1], cols[2], " | ".join(cols[3:])
        refs = re.findall(r"`([^`]+)`", ver)
        other = re.sub(r"`[^`]+`", "", ver).strip(" ,;")
        rows.append({"id": rid, "req": req, "impl": impl, "refs": refs, "other": other})
    return rows


def run_tests():
    """Return {full_test_path: status} and {source_file: [test paths]}."""
    # One merged stream: cargo prints "Running <file>" on stderr and the test
    # results on stdout; only the merged order keeps file and tests together.
    # Plain output: CI sets CARGO_TERM_COLOR=always, and colour codes would
    # hide the "Running" lines from the parser. Strip any that remain.
    env = dict(os.environ, CARGO_TERM_COLOR="never")
    p = subprocess.run(["cargo", "test", "--workspace", "--", "--test-threads=4", "--color=never"], cwd=ROOT,
                       stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, env=env)
    out = re.sub(r"\x1b\[[0-9;]*m", "", p.stdout)
    results, by_file, current = {}, {}, None
    for line in out.splitlines():
        m = re.match(r"\s*Running (\S+) \(", line)
        if m:
            current = m.group(1)
            continue
        m = re.match(r"test (\S+) \.\.\. (ok|FAILED|ignored)", line)
        if m:
            results[m.group(1)] = m.group(2)
            by_file.setdefault(current, []).append(m.group(1))
    if not results:
        sys.exit("no test results parsed; cargo output:\n" + out[-3000:])
    return results, by_file


def resolve(ref, results, by_file):
    """Tests matching one reference: list of (name, status)."""
    if ref.endswith(".rs"):
        for f, names in by_file.items():
            if f and f.endswith(ref):
                return [(n, results[n]) for n in names]
        return None
    if "::" in ref:
        hits = [(n, s) for n, s in results.items() if (n + "::").startswith(ref + "::") or n.startswith(ref + "::")]
        return hits or None
    hits = [(n, s) for n, s in results.items() if n.split("::")[-1] == ref]
    return hits or None


def is_test_ref(ref, results, by_file):
    return ref.endswith(".rs") or "::" in ref or any(n.split("::")[-1] == ref for n in results) \
        or re.fullmatch(r"[a-z][a-z0-9_]+", ref) is not None


def main():
    results, by_file = run_tests()
    rows = hlr_rows()
    referenced, bad = set(), 0
    table = []
    for r in rows:
        tests, missing, failed = [], [], []
        for ref in r["refs"]:
            if not is_test_ref(ref, results, by_file):
                continue  # e.g. a file name used as review evidence
            hits = resolve(ref, results, by_file)
            if hits is None:
                missing.append(ref)
                continue
            for n, s in hits:
                referenced.add(n)
                tests.append(n)
                if s != "ok":
                    failed.append(n)
        if missing or failed:
            status = "FAIL" if failed else "MISSING"
            bad += 1
        elif tests:
            status = "PASS" + (" + review/bench" if r["other"] else "")
        else:
            status = "REVIEW/BENCH"
        table.append({**r, "tests": tests, "missing": missing, "failed": failed, "status": status})

    os.makedirs(OUT, exist_ok=True)
    unref = sorted(n for n in results if n not in referenced)
    passed = sum(1 for s in results.values() if s == "ok")
    with open(os.path.join(OUT, "trace-matrix.md"), "w", encoding="utf-8") as f:
        f.write("# Trace matrix: high-level requirements -> tests\n\n")
        f.write(f"Tests run: {len(results)}, passed: {passed}. Requirements: {len(table)}, "
                f"with problems: {bad}.\n\n")
        f.write("| HLR | Status | Tests | Other evidence |\n|---|---|---|---|\n")
        for t in table:
            tests = "<br>".join(f"`{n}`" for n in t["tests"]) or "–"
            extra = "; ".join(filter(None, [t["other"],
                                            ("MISSING: " + ", ".join(t["missing"])) if t["missing"] else "",
                                            ("FAILED: " + ", ".join(t["failed"])) if t["failed"] else ""])) or "–"
            f.write(f"| {t['id']} | **{t['status']}** | {tests} | {extra} |\n")
        f.write(f"\n## Tests not referenced by any HLR ({len(unref)})\n\n")
        f.write("Supporting / lower-level tests. Review periodically whether an HLR is missing.\n\n")
        for n in unref:
            f.write(f"- `{n}` ({results[n]})\n")
    with open(os.path.join(OUT, "trace-matrix.csv"), "w", newline="", encoding="utf-8") as f:
        w = csv.writer(f)
        w.writerow(["hlr", "status", "tests", "missing", "failed", "other_evidence", "requirement"])
        for t in table:
            w.writerow([t["id"], t["status"], " ".join(t["tests"]), " ".join(t["missing"]),
                        " ".join(t["failed"]), t["other"], t["req"]])
    print(f"trace matrix: {len(table)} HLRs, {bad} with problems; {passed}/{len(results)} tests passed; "
          f"{len(unref)} tests not referenced -> {OUT}/trace-matrix.md")
    failed_tests = [n for n, s in results.items() if s == "FAILED"]
    if bad or failed_tests:
        for t in table:
            if t["missing"] or t["failed"]:
                print(f"  {t['id']}: missing={t['missing']} failed={t['failed']}")
        sys.exit(1)


if __name__ == "__main__":
    main()
