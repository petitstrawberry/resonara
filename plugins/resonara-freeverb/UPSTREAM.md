# Source provenance

This directory is the canonical source of Resonara’s bundled Freeverb DSP and
ScarletUI editor. It is maintained in the Resonara repository.

The DSP is adapted from trevyn/freeverb, MIT (Ian Hobson, 2018):
https://github.com/trevyn/freeverb/tree/d89365ce8381751bea6b0e85294b7f6da3ad98ef
The original reference source and license are preserved in vendor/freeverb.

The CLAP adapter and rotary editor originated in Resonara (MIT).
Freestanding CLAP bindings and their CLAP license are preserved in vendor/clap-sys.

Earlier revisions were also published separately as freeverb-scarlet and
resonara-freeverb. That repository is historical; this directory is no longer
an upstream snapshot and builds do not fetch it.
