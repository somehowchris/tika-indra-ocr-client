# Exact derived evaluation inputs

`legal-raster/` preserves eight public European Union legal-page PNGs and their
eight unmodified PDF-text candidates. These are frozen inputs and reference
construction materials, not OCR predictions. The original case manifest and
reference-overlay locks remain the admission authority.

The [fixture manifest](legal-raster.json) records every byte count and SHA-256,
source PDF identity and URL, legislation cluster, language, original page,
dimensions, rendering command, toolchain closure and source reuse conditions.
The eight input PNGs total 10,001,367 bytes. Individual files make their contents
and provenance easier to inspect than another archive format.

The original rasterization used Poppler 26.05.0 with the recorded font, image
codec and compression dependencies. A newer or different host renderer cannot
be assumed to produce the same pixels or PNG bytes. Hosted reproduction copies
these exact admitted candidates to their declared corpus paths; the Rust
preparer verifies and reuses them. It still constructs the audited legal text
references by applying only the already frozen visible-hyphen overlays. Other
source datasets are acquired and prepared through the pinned Rust toolchain.

These files preserve the original source selection. Replacing a difficult
image, editing a reference after prediction, or accepting a different hash is
not a reproduction. Original PDFs and independently audited official HTML
remain identified in the acquisition locks and legal audit.

**Attribution:** European Union, Official Journal of the European Union. The
per-file records identify the CELEX act, language and original page. The source
legal documents follow the [EUR-Lex document reuse conditions](https://eur-lex.europa.eu/content/legal-notice/legal-notice.html),
including document-specific exceptions; the separate editorial-content licence
is not treated as the licence for every legal act. Rasterization and the frozen
visible-hyphen reference changes are disclosed transformations. Experimental
OCR output is not an official legal text.
