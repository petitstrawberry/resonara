# Mixer scale reference

The native mixer scale is informed by the user's requested reference,
[Spectrum](https://github.com/petitstrawberry/spectrum), inspected on 2026-10-01
at default-branch (`dev`) commit
`965f8dbf328c3fb0c0bbc465cb84974cd0f6def9`.

## Reference geometry

Spectrum has **different** gain-fader and signal-meter scales. Both interpolate
linearly between the following anchors; the percentage is measured upward from
the bottom. For a vertical drawable range `top..bottom`, the corresponding
coordinate is `bottom - percentage / 100 * (bottom - top)`.

| Gain dB | Fader % | Peak dBFS | Meter % |
| ---: | ---: | ---: | ---: |
| −100 | 0 | −60 | 0 |
| −40 | 8.2 | −50 | 8.2 |
| −30 | 12.3 | −45 | 15.7 |
| −20 | 20.9 | −40 | 23.1 |
| −15 | 29.1 | −35 | 30.2 |
| −10 | 39.9 | −30 | 37.7 |
| −6 | 48.5 | −24 | 46.6 |
| −3 | 61.2 | −21 | 53.4 |
| 0 | 74.3 | −18 | 60.1 |
| +3 | 86.9 | −15 | 66.8 |
| +6 | 100 | −12 | 73.5 |
| | | −9 | 79.9 |
| | | −6 | 86.6 |
| | | −3 | 93.3 |
| | | 0 | 100 |

The inverse fader mapping interpolates the same anchors with axes exchanged.
Spectrum represents its lowest fader endpoint as silence (`−∞`) rather than
presenting `−100` as a normal tick. Its full fader tick set is +6, +3, 0, −3, −6,
−10, −15, −20, −30, −40 and −∞. Its meter displays attenuation labels
0, 6, 12, 18, 24, 30, 40, 50 and 60, corresponding to nonpositive dBFS.

Source: [scale helpers and ticks, lines 36–214](https://github.com/petitstrawberry/spectrum/blob/965f8dbf328c3fb0c0bbc465cb84974cd0f6def9/src/ui/MixerPanel.tsx#L36-L214).

Amplitude is converted to dB using `20 * log10(abs(amplitude))`. Spectrum clamps
the meter's display range to −60…0 dBFS before mapping it. Its visual smoothing
uses a fixed 0.35 blend per animation frame; that implementation detail is
frame-rate dependent and is not a calibrated audio-meter ballistics standard.
Source: [meter drawing, lines 570–606](https://github.com/petitstrawberry/spectrum/blob/965f8dbf328c3fb0c0bbc465cb84974cd0f6def9/src/ui/MixerPanel.tsx#L570-L606).

The reference comments describe a Logic Pro X scale. This document records
Spectrum's actual numbers, not a claim that Apple specifies or certifies them.
Native tick density may be reduced at short control heights to keep labels
readable; the underlying transfer functions must not change with tick density.

## Native interaction

The native left gain labels are clickable: +6, 0, −6, −18 and −48 dB map
directly to linear gain, while −∞ sets zero. Selection chooses the nearest
painted label center at compact heights. The track/thumb still supports
continuous pointer and keyboard adjustment; double-clicking the thumb resets
to unity. Right-hand Peak dBFS meter labels are read-only. Track and Master use
the same interaction and preserve one history entry per gesture.

## Attribution and licensing evidence

Spectrum is authored by petitstrawberry. Its
[README](https://github.com/petitstrawberry/spectrum/blob/965f8dbf328c3fb0c0bbc465cb84974cd0f6def9/README.md#license)
and [Rust package metadata](https://github.com/petitstrawberry/spectrum/blob/965f8dbf328c3fb0c0bbc465cb84974cd0f6def9/src-tauri/Cargo.toml#L1-L8)
declare MIT. The complete repository tree at that revision contains no LICENSE
file, despite the README link; no missing copyright notice or license text has
been fabricated here. The anchor data and geometry are documented as design
reference for Resonara's independent ScarletUI implementation. No Spectrum
runtime dependency or React component is added to Resonara.
