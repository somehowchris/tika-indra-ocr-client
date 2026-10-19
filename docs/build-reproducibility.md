# Reproducible application compilation

The application compiler stages in `docker/native.Dockerfile` set
`SOURCE_DATE_EPOCH=0`. This is fixed
reproducibility metadata, not the real build time. Source commits, release runs
and their timestamps remain separately recorded. The fixed epoch avoids
invalidating dependency reuse for every application-only edit.

This addresses an observed failure in the original frozen WP1 proof:
`2193324` versus candidate Dockerfile/selector `29695e8`, with the same admitted
SDK `sha256:3a98d2e…`, produced four identical test executables and three unequal
production executables. All three unequal `.rodata` printable-string multisets
differed only in mimalloc's compile clock. The archived, identical
`libmimalloc-sys 0.1.49` source compiles mimalloc v3's `static.c`; its
`mi_options_print_out` embeds `__DATE__` and `__TIME__`. The
[pinned build script](https://github.com/purpleprotocol/mimalloc_rust/blob/abcd2be6171e89190f8087fcaccf001d8bd5bc96/libmimalloc-sys/build.rs)
selects v3 by default. [GCC 14.2 documents](https://gcc.gnu.org/onlinedocs/gcc-14.2.0/cpp/Environment-Variables.html)
how the explicit epoch fixes both macros.

The differences include real `.text`, `.rela.dyn`, `.rodata`, build ID and, for
OCR/server, `malloc_hook` bytes. Full executable-section disassembly shows that every changed instruction
differs only in its RIP-relative reference into `.rodata`: 657 in engine
acceptance, 12,494 plus 2 hook instructions in OCR, and 24,125 plus 4 hook
instructions in the server. All changed `.rela.dyn` entries preserve location
and type, changing only the addend into `.rodata` (1,209 / 968 / 4,838 entries). These binaries have not
been normalized or declared identical. The original failed attempt and plan
remain intact.

A small check compiled the archived C source with the exact admitted GCC
executable. Two wall-clock objects differed; two epoch-zero objects had exactly
the same SHA-256. Its flags are explicitly recorded and are not a capture of the
original Cargo build script's complete compiler arguments. This proves the C
clock mechanism; the completed three-arm proof below supplies the application
byte and material evidence. [Compact evidence](../evaluation/build-cost/evidence/mimalloc-build-clock-2026-10-19/)
retains the hashes, section differences and probe recipe.

The shared application stage follows the existing native-prefix delimiter, so
its addition does not change the native SDK recipe. Engine, server, diagnostic
profile and Chef stages inherit it. Chef records the pinned epoch in its key,
checks the actual planner environment, and rejects old keys without that input.
A new compatible Chef artifact is required; the native SDK can remain the same.
Routine admission compares this contract before any Chef restore. A legitimate
Rust-only change can explicitly select SDK-only compilation.

The predeclared [epoch-zero amendment](../evaluation/build-cost/evidence/seven-role-epoch0-amendment-2026-10-19.json)
applies identical rendered-recipe injection to both original frozen arms and
adds an independent baseline repeat. It changes observable allocator date/time
metadata. It does not establish equality to the original wall-clock executables.
The source contexts, original four Cargo invocations and their flags, and the
candidate's two-file source delta remain unchanged. Compiler stages are excluded
from BuildKit reuse for all three arms. All seven raw executable hashes must
match across all arms, static ELF checks must pass, candidate materials must
contain only the original declared source delta, and repeat materials and
configuration must match exactly. No output normalization is permitted.

```sh
python3 scripts/prove-frozen-wp1.py prepare --application-epoch-zero \
  --sdk-layout /absolute/path/to/admitted-3a98-oci-layout \
  --output /absolute/path/to/new-exclusive-proof-directory
python3 scripts/prove-frozen-wp1.py run \
  --output /absolute/path/to/new-exclusive-proof-directory
```

The completed amendment at controller `c935015` exited successfully. All seven
raw executable hashes match across baseline, candidate and the independently
recompiled baseline; all 21 static ELF checks pass. Both original source contexts
remain frozen (334 required baseline files, 335 candidate files including the
new selector). Complete source, vendor, native, licence and relink materials
contain only the two original candidate source differences; the baseline repeat
has no material difference. All 31 required native notices and configuration/
OpenAPI bytes pass. Each of the six compiler/export phases records the explicit
compiler-stage cache exclusion and two actual mimalloc profile compilations.
[The result receipt](../evaluation/build-cost/evidence/mimalloc-build-clock-2026-10-19/epoch-zero-proof-result.json)
binds all raw hashes, controls and the complete retained file-inventory digest.
No executable bytes were normalized, and no native source build was executed.

This is an executable/material content proof on a shared builder. It does not
establish fresh-runner restoration, lifecycle behavior, OCR quality or speedup.
Trusted release `39213042394` uses older source `5ae8b82` without this fix.
It failed at Kubernetes CRI preflight before SDK publication or the independent
seven-role proof. That proof was skipped; the run supplies no observed clock-
related proof failure and no signed SDK eligible for experimental reuse.
