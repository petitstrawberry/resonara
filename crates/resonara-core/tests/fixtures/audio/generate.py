#!/usr/bin/env python3
"""Regenerate small synthetic decoder fixtures. Requires Python 3 and FFmpeg.

No recordings or third-party samples are used. Run from any working directory.
The source PCM is deterministic; encoded bytes can vary by FFmpeg version.
"""
import math
from pathlib import Path
import struct
import subprocess
import wave

ROOT = Path(__file__).resolve().parent
SOURCE = ROOT / "source.wav"
with wave.open(str(SOURCE), "wb") as wav:
    wav.setparams((2, 2, 44100, 5292, "NONE", "not compressed"))
    wav.writeframes(b"".join(
        struct.pack("<hh", round(12000 * math.sin(math.tau * 440 * i / 44100)),
                    round(4000 * math.sin(math.tau * 880 * i / 44100)))
        for i in range(5292)
    ))

FORMATS = [
    ("stereo.flac", ["-c:a", "flac"]),
    ("stereo.aiff", ["-c:a", "pcm_s16be"]),
    ("stereo.caf", ["-c:a", "pcm_s16le"]),
    ("alac.m4a", ["-c:a", "alac"]),
    ("aac.m4a", ["-c:a", "aac", "-b:a", "128k"]),
    ("stereo.aac", ["-c:a", "aac", "-b:a", "128k"]),
    ("stereo.mp3", ["-c:a", "libmp3lame", "-b:a", "128k"]),
    ("stereo.ogg", ["-c:a", "libvorbis", "-q:a", "4"]),
    ("mono.flac", ["-ac", "1", "-c:a", "flac"]),
]
for filename, options in FORMATS:
    subprocess.run([
        "ffmpeg", "-nostdin", "-hide_banner", "-loglevel", "error", "-y",
        "-i", str(SOURCE), "-map_metadata", "-1", *options, str(ROOT / filename)
    ], check=True)
    print(f"Generated {filename} ({(ROOT / filename).stat().st_size} bytes)")
