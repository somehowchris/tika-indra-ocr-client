Independent reviewer: `/root/native_engine/lab_audit`

This record was appended after the scientific freeze. It preserves the complete independent audit report below verbatim; no frozen file, reference, README or input was changed when this record was created.

---

## Independent audit result: pass within the frozen crop scope

**I found no discrepancy requiring an input, reference or protocol edit.** The frozen files should remain unchanged. I performed this audit read-only: I viewed both complete PNGs at their original dimensions, compared every visible text line with the canonical and raw extraction references, checked the selected HTML paragraphs against the retained full HTML sources, inspected the source licence evidence, and verified the recorded hashes. I did not run OCR, rerender the PDFs, acquire sources or modify files.

### Visual and reference findings

| Case | Complete visible material reviewed | Finding |
|---|---|---|
| `hops-retention-crop` | All six lines of the retention-index and mass-spectrometry paragraph | Complete paragraph; no clipped glyphs, omitted visible text or reading-order discrepancy. The visible `C8`, `C20`, `(IRL),33`, `NIST`, `Wiley` and `FFNSC` are preserved. |
| `chlorins-synthesis-crop` | Both headings and all 26 body lines across three preparation paragraphs—28 nonempty lines total | All visible text is covered in order. No clipped glyphs or omitted visible lines. The headings, physical `metil-` line-end hyphen, accented and unaccented printed spellings, quantities, chemical symbols and all citation digits are preserved. |

For the chlorins crop, I specifically checked the printed molecule ranges `1-3` and `4-6`, compound identifiers `(1)`, `(2)` and `(3)`, `feoforbídeo-a`, literal `metil-feoforbideo`, `purpurina p-18`, both `10%` concentrations, both `48 h` durations, `HCl`, `KOH`, `4.8 g`, `16 mL`, `1 h`, `pH` range `2-4`, and citations `29`, `38` and `6,39`. The reference preserves these as printed, including the wording `foi obtida`.

There is visible white space around the text on every crop boundary. As a supporting pixel check, the nonwhite bounding boxes are:

- Hops: `(11, 9, 1022, 273)` inside the `1030 × 287` image.
- Chlorins: `(18, 26, 1029, 1382)` inside the `1046 × 1404` image.

The PNGs are RGB-encoded, but their three channels are identical throughout; the recorded description of grayscale image content is accurate.

**Completeness applies to the stated crop contents:** one full paragraph for hops, and two headings plus three full paragraphs for chlorins. This does not establish that an entire original methods section, full page or document was included.

### Canonical transformations and independent representation

Both canonical references reproduce exactly from their adjacent `.pdf-text.txt` files under the frozen rules: remove the form-feed and horizontal extraction indentation, retain physical line breaks and the terminal newline, and apply the single declared chlorins superscript-spacing overlay.

The following checks all passed:

- Removing the extra Poppler spaces in `literatura.     29` produces the canonical `literatura.29` without losing the citation.
- The canonical chlorins reference retains `metil-\nfeoforbídeo`.
- Joining that physical line-end hyphen only for the HTML comparison, reflowing whitespace with NFC, and normalizing the final HTML citation separator from `6, 39` to `6,39` yields exact semantic agreement.
- Each retained `.independent-html.txt` excerpt matches the corresponding complete paragraph or paragraphs in the locked full HTML source.
- Both chlorins headings also appear in the retained HTML source.
- The prepared references are byte-identical to their repository `references/` copies, and the prepared raw excerpts are byte-identical to their repository `audit/` copies.

The documentation correctly limits “independent” here to **another representation of the same article**. It is not an independently written expert transcription, and this audit does not certify chemical correctness.

### Language, rights and reporting scope

I found **no English word passage, expansion or sentence inside either crop**. The Portuguese prose and headings are complete within the stated windows. The disclosed names/acronyms `NIST`, `Wiley` and `FFNSC`, chemical symbols, and printed Latin binomial *Spirulina máxima* are present and consistent with the declared non-English-prose classification.

I independently counted **324 NFC reference letters for hops and 1,362 for chlorins, totaling 1,686**. The zero-English count is justified under the explicitly stated language policy.

The retained full-page PDF text corroborates the disclosure that **both original source pages contain English**:

- Hops PDF page 3 contains `baby hops` and the expanded principal-component-analysis label.
- Chlorins PDF page 2 contains expanded HOMO/LUMO molecular-orbital terms.

The manifest identifies both admitted inputs as `oracle_crop`, and the README and protocol explicitly exclude full-page performance claims. The `0/2` crop-derived original-page-group count must continue to be distinguished from **`2/2` English-containing full source pages**.

For rights, I inspected both article-specific `section.documentLicense` blocks in the retained SciELO HTML. Each directly links `https://creativecommons.org/licenses/by/4.0/` and describes the article’s Attribution licence. The retained full PDF text ends with a Creative Commons Attribution notice for each article. I found no contradictory noncommercial notice in the inspected retained article HTML/PDF text. The recorded authors, DOI attribution and adaptation disclosure therefore have article-specific supporting evidence; they are not inferred solely from a journal footer.

The comparison contract is appropriately constrained:

- Two cases from two distinct article clusters, analyzed separately from the original study.
- All eight arm definitions and the resource-budget object match the parent protocol.
- CPU budgets remain separate.
- Three repetitions are technical repeats.
- Descriptive case/cluster results only, with no confidence intervals or population superiority claim from two articles.
- First-attempt quality, failures, incomplete runs, cold readiness and warmed measurements remain distinguished.
- Output-based replacement or reference repair is prohibited.

### Exact input and reference hashes inspected

All eight items below were read or viewed and independently hashed. All hashes match the frozen records, and all corresponding repository reference/audit copies match byte-for-byte.

| File in `lab-supplement/prepared/` | Bytes | SHA-256 |
|---|---:|---|
| `hops-retention-crop.png` | 81,175 | `d627da6a0986507e7dec9b442224c37b4e90826a8d206fef347a82d29a01baaa` |
| `hops-retention-crop.txt` | 410 | `d2d4e7f1bc3814c25ebb2294984d081e9f306de06537fde3c77356861c08dfaf` |
| `hops-retention-crop.pdf-text.txt` | 415 | `ee374188f496cf7cb486c1976d264b737b56c305d388d7a84345d4d4cdefd92d` |
| `hops-retention-crop.independent-html.txt` | 410 | `17a162324ce6f4e14ac598a9941b5935feccd92b895542e6e48996e51d470a72` |
| `chlorins-synthesis-crop.png` | 357,696 | `264b2a5d7f0d56f878c6e5bf8f3f54e878ec51de5f0997e7150ee14680d88891` |
| `chlorins-synthesis-crop.txt` | 1,784 | `f4d0ed0e5aea2d94343c9cc103d27ab0cee9eac961d03f627111c9f2f7ff0e74` |
| `chlorins-synthesis-crop.pdf-text.txt` | 1,805 | `4642dbac6e1159554c4d24e6b9feb9904d700e96a2f32e3f156e48b92d266456` |
| `chlorins-synthesis-crop.independent-html.txt` | 1,725 | `d5bbbca3b25d213694e1d32b3f7676afae0f92b2d84f1f55144174a86f09a9d7` |

### Freeze and source integrity

**All 13 entries in `freeze.json` passed their byte-length and SHA-256 checks. All six assets in the source acquisition lock also passed.**

| Identity | Independently computed SHA-256 |
|---|---|
| Supplement `freeze.json` | `61fb0d17e7dbb81f1a3e4ba802a82c24675a4c7563995113545e3b4d0e3548cf` |
| Supplement `corpus-frozen.json` | `a2619116c3b2423f8eba9227e67b52258577bd249173186ef565cfe5cffbed66` |
| Supplement `protocol.json` | `a4a71aaa1a9ef00dc82c27e7c074035a73a47195691a5a26eba62eb09a43210f` |
| Supplement `README.md` reviewed | `ad1d3c6072798f431815c9b336599f8d91c30517e57d6886bc31419583b7dd45` |
| Supplement `audit/license-evidence.json` | `d291e84dce821b0bf0eeeb7f11078ebd81a442cb645ce80e91252681a2328924` |
| Original 41-case `evaluation/corpus-frozen.json` | **`073de0a6c6dba809c242c5771814fb8218196bf2743b2836b60b044607216843`** |
| Parent `evaluation/protocol.json` | `04445f89e38f6b09e810e01975ab706f56816706f3ed5ea30fe4c802986fa3c8` |

The original corpus therefore remains byte-identical to the required identity. The parent-protocol hash binding is also correct. The README was inspected at the identity above; it is not itself one of the 13 entries in the scientific freeze.

| Retained source asset | SHA-256 |
|---|---|
| `sources/hops.pdf` | `b09904825af6341f75b7bc892f8f5397bbef520df82463e7a10b71856f2f5691` |
| `sources/hops.html` | `808ea90f15ff97a398d05c854480f409fddd0d24db928885c1c834a3ce7cb01e` |
| `sources/chlorins.pdf` | `6e4e5b364f95f2e1226f098432e592c6e266055ea1cefe61ddf40cd48518bb33` |
| `sources/chlorins.html` | `11b2ffccbfa1196079433df920ee557e876eca0aff48caeb176aa117fc0e0a33` |
| `sources/chlorins-pdf-route.html` | `a7b8bd4ab1e9de250bbd965968a22ca329bc1969f4c6d893f087466b9cf7e8e9` |
| `sources/chlorins-scielo.html` | `2fbd907bc279f415684ed430dc0c255cb99c8ca97671ac83db98bb0af7d2bf12` |

### Audit limits and disposition

This is an independent AI-assisted visible-text, provenance and protocol audit. It does not establish unknown model-training overlap, domain-expert chemical validation, or historical absence of inference solely from the files. The no-prior-OCR state is explicitly recorded in the freeze and was supplied as task context; **this audit itself performed no inference**.

**Edits needed: none.** Preserve the scientific freeze and canonical references. Record this audit separately, linked to the freeze and input/reference identities above.
