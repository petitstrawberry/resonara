#!/usr/bin/env python3
"""Build and audit Scarlet Freeverb using the shared freestanding CLAP builder."""
from pathlib import Path
import subprocess
import sys
root = Path(__file__).resolve().parent
raise SystemExit(subprocess.call([sys.executable, str(root.parent / "resonara-gain/build.py"),
    "--plugin-root", str(root), *sys.argv[1:]]))
