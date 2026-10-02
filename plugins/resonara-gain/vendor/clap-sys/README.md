# Vendored no_std CLAP ABI subset

Derived from clap-sys 0.5.0, by Micah Johnston and Robbert van der Helm,
https://github.com/micahrj/clap-sys, under its MIT license (LICENSE-MIT).
The original crate's exact C-layout types and constants are retained for
the modules used by this effect. The only API source edits replace `std::`
with `core::`; the crate root adds `#![no_std]`, permits the upstream legacy-constant
spelling lint, and omits unused modules. Rustfmt normalizes formatting.
No DSP, host behavior, or commercial plug-in code is copied.

The CLAP API is https://github.com/free-audio/clap (MIT), version 1.2.2 in
these bindings. `src/tests.rs::abi_matches_registry_bindings` in the parent effect compares the vendored type sizes, alignments,
and offsets with registry clap-sys 0.5.0.
