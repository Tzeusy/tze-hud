#!/usr/bin/env python3
"""Check that dev-mode is not in default features of release-critical workspace members.

Reads cargo metadata JSON from stdin and exits non-zero if dev-mode appears in
the default features of tze_hud_runtime, vertical_slice, or integration.

Used by the justfile dev-mode-guard recipe and mirrors the metadata-check step
in the CI dev-mode-guard job (.github/workflows/ci.yml).
"""
import json
import sys

meta = json.load(sys.stdin)
fail = False
for pkg in meta["packages"]:
    if pkg["name"] in ("tze_hud_runtime", "vertical_slice", "integration"):
        features = pkg.get("features", {})
        default_features = features.get("default", [])
        if "dev-mode" in default_features:
            print(f"FAIL: dev-mode is in default features of {pkg['name']}")
            fail = True
        else:
            print(f"PASS: dev-mode is not in default features of {pkg['name']}")
sys.exit(1 if fail else 0)
