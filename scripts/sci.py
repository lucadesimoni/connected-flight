#!/usr/bin/env python3
# Copyright (c) 2026 Outthinkx by Luca De Simoni. All rights reserved.
# Proprietary software, not open source. See LICENSE.
"""Software Configuration Index (SCI) for a build.

Records exactly what a release consists of: source revision, toolchain,
every dependency (version, source, checksum from Cargo.lock), and SHA-256 of
the release binaries and of the configuration and OS integration files.
Run after `cargo build --release`. Writes target/evidence/sci.{md,json}.
"""
import hashlib, json, os, subprocess, sys, tomllib, datetime

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = os.path.join(ROOT, "target", "evidence")
BINARIES = ["cfbd", "cfb-io", "cfb-maint", "cfb-release", "cfb-replay", "cfb-a429-survey", "cfb-ground"]


def sh(*cmd):
    try:
        return subprocess.run(cmd, cwd=ROOT, capture_output=True, text=True, check=True).stdout.strip()
    except Exception as e:  # noqa: BLE001 - recorded as unavailable
        return f"unavailable ({e.__class__.__name__})"


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def files_under(*dirs):
    out = []
    for d in dirs:
        for base, _, names in os.walk(os.path.join(ROOT, d)):
            for n in sorted(names):
                p = os.path.join(base, n)
                out.append((os.path.relpath(p, ROOT), sha256(p)))
    return sorted(out)


def main():
    lock = tomllib.load(open(os.path.join(ROOT, "Cargo.lock"), "rb"))
    deps = [{"name": p["name"], "version": p["version"], "source": p.get("source", "workspace"),
             "checksum": p.get("checksum", "")} for p in lock["package"]]
    bins = []
    for b in BINARIES:
        p = os.path.join(ROOT, "target", "release", b)
        if os.path.exists(p):
            bins.append({"binary": b, "sha256": sha256(p), "bytes": os.path.getsize(p)})
        else:
            print(f"warning: target/release/{b} not built", file=sys.stderr)
    sci = {
        "generated_utc": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "git_commit": sh("git", "rev-parse", "HEAD"),
        "git_describe": sh("git", "describe", "--always", "--dirty", "--tags"),
        "git_dirty": sh("git", "status", "--porcelain") != "",
        "toolchain_file": open(os.path.join(ROOT, "rust-toolchain.toml"), encoding="utf-8").read(),
        "rustc": sh("rustc", "-V"),
        "cargo": sh("cargo", "-V"),
        "binaries": bins,
        "dependencies": deps,
        "config_and_os_files": [{"path": p, "sha256": h} for p, h in files_under("config", "os", "ground")],
    }
    os.makedirs(OUT, exist_ok=True)
    json.dump(sci, open(os.path.join(OUT, "sci.json"), "w"), indent=2)
    with open(os.path.join(OUT, "sci.md"), "w", encoding="utf-8") as f:
        f.write("# Software Configuration Index\n\n")
        for k in ("generated_utc", "git_commit", "git_describe", "git_dirty", "rustc", "cargo"):
            f.write(f"- **{k}**: `{sci[k]}`\n")
        f.write("\n## Binaries\n\n| Binary | Bytes | SHA-256 |\n|---|---|---|\n")
        for b in bins:
            f.write(f"| {b['binary']} | {b['bytes']} | `{b['sha256']}` |\n")
        f.write(f"\n## Dependencies ({len(deps)})\n\n| Crate | Version | Source | Checksum |\n|---|---|---|---|\n")
        for d in deps:
            f.write(f"| {d['name']} | {d['version']} | {d['source'].split('+')[0]} | `{d['checksum'][:16]}…` |\n")
        f.write("\n## Configuration and OS files\n\n| File | SHA-256 |\n|---|---|\n")
        for e in sci["config_and_os_files"]:
            f.write(f"| {e['path']} | `{e['sha256'][:16]}…` |\n")
    print(f"SCI: commit {sci['git_commit'][:10]} dirty={sci['git_dirty']}, {len(bins)} binaries, "
          f"{len(deps)} dependencies -> {OUT}/sci.md")
    if sci["git_dirty"]:
        print("note: working tree has uncommitted changes; a release SCI must come from a clean tree")


if __name__ == "__main__":
    main()
