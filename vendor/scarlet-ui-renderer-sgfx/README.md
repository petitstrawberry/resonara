# ScarletUI SGFX renderer patch

Source: `petitstrawberry/scarlet-ui`, commit
`91238ed12c3379c0aea7e7ba86e73139cfb1f550`, package
`crates/scarlet-ui-renderer-sgfx`. The upstream license is included.

This package is vendored to fix canvas target exhaustion during sidebar and
window resizing. Upstream retains a target for every `(handle, width, height)`;
three waveforms exceed its 32-target limit after ten distinct width changes.
Tracked in [issue #30](https://github.com/petitstrawberry/scarlet-ui/issues/30);
the fix and regression tests are submitted in
[PR #31](https://github.com/petitstrawberry/scarlet-ui/pull/31).

Only `src/lowering.rs` differs from that source. Targets retain one cache entry
per concurrent canvas size, grow allocated texture dimensions geometrically, and reuse that
capacity for subsequent sizes. Rendering uses the requested viewport dimensions;
texture coordinates crop the capacity padding so resizing preserves waveform
geometry and aspect ratio. A size change invalidates the cached rendered content
even when its frame revision has not changed.

Canvas passes explicitly set the requested viewport and scissor: WGPU's default
viewport covers the entire allocated texture, which otherwise stretches and
clips the lower stereo lane. Clears cover the full capacity so depth-tested
canvases do not require unsupported partial depth clears.

Immutable resource definitions are never overwritten or released while prior GPU
work may still reference them. Growing beyond capacity defines a new texture;
the number of definitions depends on capacity growth, not the number of resize
events. Other ScarletUI crates and the SGFX core remain at their existing pins.

The Cargo manifest replaces the upstream relative core path with the same pinned
Git source. Remove this patch when the corresponding upstream fix is available.
