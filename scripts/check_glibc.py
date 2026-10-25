#!/usr/bin/env python3
"""Fail when an ELF binary needs a newer glibc than the Steam Linux Runtime provides.

Steam Linux Runtime 3.0 (sniper) ships glibc 2.31. A binary that references a
newer GLIBC_x.y symbol version fails to start inside that container.
"""
import re
import subprocess
import sys

LIMIT = (2, 31)


def required(path):
    dump = subprocess.check_output(["objdump", "-T", path], text=True)
    versions = {tuple(map(int, v.split("."))) for v in re.findall(r"GLIBC_(\d+\.\d+(?:\.\d+)?)", dump)}
    return max(versions) if versions else (0, 0)


def main(paths):
    failed = False
    for path in paths:
        need = required(path)
        ok = need[:2] <= LIMIT
        failed |= not ok
        print(f"{path}: needs GLIBC {'.'.join(map(str, need))} ({'ok' if ok else 'too new'}; limit {LIMIT[0]}.{LIMIT[1]})")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
