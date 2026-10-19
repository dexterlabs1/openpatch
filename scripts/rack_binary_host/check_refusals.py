#!/usr/bin/env python3
"""Check real companion profile/restore refusals; no audio device or user state."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import struct
import subprocess
import tempfile


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("companion", "sdk", "plugin", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    output = Path(tempfile.mkdtemp(prefix="refusals-", dir=args.output.resolve()))
    watched = [args.companion, args.sdk / "libRack.dylib", args.plugin / "plugin.dylib", args.plugin / "plugin.json", Path(__file__)]
    before = {str(p.resolve()): digest(p) for p in watched}
    fixture = json.loads((Path(__file__).parent / "fixtures/xfade-default-state.json").read_text())
    results = []
    cases = [("foreign_profile", 1, 2, 5), ("unknown_profile", 99, 2, 3),
             ("legacy_protocol", 2, 1, 3), ("fractional_switch", 2, 2, 5),
             ("curve_out_of_range", 2, 2, 5), ("reserved_header", 2, 2, 3)]
    for name, profile, version, fault in cases:
        directory = output / name
        directory.mkdir(mode=0o700)
        user = directory / "private-user"
        user.mkdir(mode=0o700)
        state = json.loads(json.dumps(fixture))
        if name == "fractional_switch":
            state["params"][2]["value"] = 0.5
        if name == "curve_out_of_range":
            state["params"][1]["value"] = -0.25
        header = bytearray(128 + 4 * 34368)
        header[:8] = b"OPHST002" if version == 2 else b"OPHST001"
        for offset, value in [(8, version), (12, len(header)), (16, 128), (20, 48000),
                              (24, 4), (28, 64), (56, 3), (60, 1), (64, 16), (84, 6), (88, profile)]:
            struct.pack_into("<I", header, offset, value)
        for offset in (32, 40, 48):
            struct.pack_into("<Q", header, offset, 1)
        if name == "reserved_header":
            header[92] = 1
        transport = directory / "transport"
        transport.write_bytes(header)
        transport.chmod(0o600)
        seed = directory / "state-in.json"
        seed.write_text(json.dumps(state))
        env = {k: v for k, v in os.environ.items() if not k.startswith("DYLD_")}
        env["DYLD_LIBRARY_PATH"] = str(args.sdk.resolve())
        with (directory / "stdout").open("w") as stdout, (directory / "stderr").open("w") as stderr:
            completed = subprocess.run([str(args.companion.resolve()), str(args.plugin.resolve()), str(user),
                                        str(transport), str(seed), str(directory / "state-out.json")],
                                       env=env, stdout=stdout, stderr=stderr, timeout=10, check=False)
        # Only inspect shared bytes after child exit; no non-atomic concurrent reads.
        returned = transport.read_bytes()
        status, _, error = struct.unpack_from("<III", returned, 68)
        passed = completed.returncode == 1 and status == 2 and error == fault
        results.append({"case": name, "passed": passed, "exit": completed.returncode, "status": status, "fault": error})
    passed = all(r["passed"] for r in results) and all(digest(Path(p)) == h for p, h in before.items())
    report = {"passed": passed, "scope": "real companion refuses unsupported headers/profiles/full snapshots before processing", "inputs_sha256": before, "cases": results}
    (output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"passed": passed, "cases": len(results), "evidence": str(output)}))
    return 0 if passed else 1


if __name__ == "__main__":
    raise SystemExit(main())
