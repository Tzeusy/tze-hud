#!/usr/bin/env python3
"""Select the first two unique logical CPUs from a Linux CPU-list string."""

from __future__ import annotations

import argparse


def select_cpu_pair(cpu_list: str) -> tuple[int, int]:
    cpus: list[int] = []
    seen: set[int] = set()
    for raw_segment in cpu_list.split(","):
        segment = raw_segment.strip()
        if not segment:
            raise ValueError("CPU-list contains an empty segment")
        bounds = segment.split("-")
        if len(bounds) == 1:
            first = last = int(bounds[0])
        elif len(bounds) == 2:
            first, last = (int(value) for value in bounds)
        else:
            raise ValueError(f"invalid CPU-list segment: {segment!r}")
        if first < 0 or last < first:
            raise ValueError(f"invalid CPU-list range: {segment!r}")
        for cpu in range(first, last + 1):
            if cpu not in seen:
                seen.add(cpu)
                cpus.append(cpu)
            if len(cpus) == 2:
                return cpus[0], cpus[1]
    raise ValueError("constrained lane requires at least two available logical CPUs")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cpu_list")
    args = parser.parse_args()
    try:
        first, second = select_cpu_pair(args.cpu_list)
    except (TypeError, ValueError) as exc:
        parser.error(str(exc))
    print(f"{first},{second}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
