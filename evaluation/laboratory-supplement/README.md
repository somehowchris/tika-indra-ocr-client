# Laboratory method crops: separate prospective supplement

This supplement freezes **two complete Portuguese method-section crops from two distinct chemistry articles**, selected and referenced before running OCR on either crop. It fills a small modern laboratory-text gap with independently checkable source material. It does not establish performance on full laboratory reports, clinical records, handwriting or a representative scientific-document population.

The original [41-case corpus](../corpus-frozen.json) remains unchanged at SHA-256 `073de0a6c6dba809c242c5771814fb8218196bf2743b2836b60b044607216843`. Do not append these cases to it or pool their outcomes with its 33 primary pages. `primary` in this separate manifest means its own two prespecified inputs; both remain explicitly labelled `oracle_crop`.

## Sources and scope

| Input | Original work | Visible task |
|---|---|---|
| `hops-retention-crop` | De Souza, Contin, Vieira and Da Costa, [Química Nova 47(3), e-20230109](https://doi.org/10.21577/0100-4042.20230109), 2024 issue; first online 10 October 2023 | Complete six-line retention-index and mass-spectrometry identification paragraph, original PDF page 3; 1,030 × 287 pixels |
| `chlorins-synthesis-crop` | Acevedo, Bilha, Pereira, Araujo and Uliana, [Química Nova 47(10), e-20240061](https://doi.org/10.21577/0100-4042.20240061), 2024; first online 24 September 2024 | Experimental heading, synthesis subheading and three complete preparation paragraphs, original PDF page 2; 1,046 × 1,404 pixels |

Both article-specific SciELO `documentLicense` sections link **CC BY 4.0**, and the corresponding PDFs state the Attribution licence on their final pages. [Licence evidence](audit/license-evidence.json) binds those article bytes and locators. The publisher PDF route, independent HTML representations, exact source bytes and hashes are retained in [the acquisition lock](acquisition/sources.json). Article-specific rights take precedence over assumptions about a journal footer. No material from the earlier noncommercial 2009 Química Nova candidate is admitted.

The adaptations are 300 dpi grayscale rendering, explicit rectangular cropping and the reference transformations below. Attribute the named authors, Sociedade Brasileira de Química / Química Nova, article DOI and original page whenever redistributing these inputs or references, and disclose the adaptations. The HTML is a separate representation of the same article, not an independently written expert transcription.

The complete original pages contain English: the hops page includes an expanded principal-component-analysis label and “baby hops”; the chlorins page includes expanded molecular-orbital terms. Those full pages are not benchmark inputs. The selected contiguous crops contain Portuguese prose and headings, with **0 English letters among 1,686 NFC reference letters**, **0/2 English-containing crop cases**, **0/2 corresponding original-page groups** and **0/2 document clusters** under the stated visible-input language policy. Report separately that **2/2 source full pages contain English**. NIST, Wiley and FFNSC are disclosed names/acronyms, chemical symbols remain chemical symbols, and *Spirulina máxima* is a Latin binomial; these are not counted as English prose. No English expansion or sentence is present inside either crop.

## Reference audit

The [complete audit](audit/reference-audit.json) compares every visible line with PDF text and the full corresponding HTML paragraphs. The first case has one complete paragraph; the second has two headings and three complete paragraphs. Selection used completeness, language, licence and source-reference agreement; OCR results played no part.

Canonical references preserve visible spelling, numerical values, punctuation, the nonempty physical text lines and the printed `metil-` line-end hyphen. PDF form-feed and horizontal extraction indentation are removed. One explicit transformation removes Poppler's extra spaces between `literatura.` and superscript `29`; the digits remain. The displayed spelling `metil-feoforbideo`, `4.8 g`, `pH` range `2-4` and all citation digits are preserved. Bold, italics and superscript positioning are flattened by a text-only reference and are not scored as typography.

Blank separator lines follow the retained PDF extractor's paragraph segmentation. In the chlorins reference, a blank line follows `literatura.29`, although the image has an ordinary line gap there; the later paragraph boundary after `bibliográfica.38` has no extra blank line. This fixed convention is disclosed in the [whitespace qualification](audit/whitespace-qualification-0001.json). It is not literal whitespace gold or a uniform typographic paragraph-spacing rule. The prespecified whitespace-preserving CER therefore includes disagreement with that reference convention; interpret it alongside the whitespace-collapsed CER. The qualification changes no input, reference byte or scoring rule.

HTML agreement uses NFC and reflowed whitespace, joins that physical line-end hyphen only for this semantic comparison, and normalizes spacing in the final `6, 39` citation separator. The canonical reference does not inherit those HTML-only changes. Raw PDF extraction and selected HTML paragraphs are preserved in `audit/`. This is an AI-assisted visible-text and scope audit, not certified expert chemistry proofreading. Model training overlap remains unknown.

## Reproduce and verify

[rendering.json](rendering.json) records the exact pixel windows, output hashes, renderer identity and extractor identity. The source renderer was `pdftoppm 26.05.0`; the reference extractor was `pdftotext 24.02.0`. These are intentionally recorded separately. A different Poppler build can change pixels or PNG encoding; a hash mismatch must fail instead of silently replacing a frozen input.

From the repository root, use a new data directory and the Rust evaluator:

```sh
cargo build --release --locked --manifest-path evaluation/runner/Cargo.toml
mkdir lab-data
evaluation/runner/target/release/indra-evaluate fetch \
  --lock evaluation/laboratory-supplement/acquisition/sources.json \
  --root lab-data --include-metadata
evaluation/runner/target/release/indra-evaluate fetch \
  --lock evaluation/laboratory-supplement/acquisition/tesseract-models.json \
  --root lab-data --include-metadata
mkdir lab-data/prepared
pdftoppm -png -gray -r 300 -f 3 -l 3 \
  -x 225 -y 1863 -W 1030 -H 287 -singlefile \
  lab-data/sources/hops.pdf lab-data/prepared/hops-retention-crop
pdftoppm -png -gray -r 300 -f 2 -l 2 \
  -x 1216 -y 1754 -W 1046 -H 1404 -singlefile \
  lab-data/sources/chlorins.pdf lab-data/prepared/chlorins-synthesis-crop
cp evaluation/laboratory-supplement/references/hops-retention-crop.txt \
  lab-data/prepared/hops-retention-crop.txt
cp evaluation/laboratory-supplement/references/chlorins-synthesis-crop.txt \
  lab-data/prepared/chlorins-synthesis-crop.txt
evaluation/runner/target/release/indra-evaluate verify \
  --manifest evaluation/laboratory-supplement/corpus-frozen.json --root lab-data
```

The existing Rust validator has verified all four input/reference assets, two original-page groups, two document clusters and the language denominators. This is an admission result; it performs no OCR. Exact source hashes deliberately reject a changed publisher HTML response. Preserve the acquired locked source bundle for later replay rather than substituting current page bytes.

The separate Portuguese-only Tesseract leaves contain `por.traineddata` and `configs/txt` for each pinned fast/best revision. They do not add files to the original multilingual leaves or change the original model lock. The executable and trained-data identities are distinct: the text configuration is the retained Tika/Tesseract contract, while the actual Tesseract executable version must still be captured from its image.

## Comparison and reporting contract

[protocol.json](protocol.json) binds this manifest, its separate Tesseract lock and both finalized Tika configuration files. Use the same eight prespecified arms: Tesseract fast PSM 3; best PSM 3, 6 and 11; Python Indra text/math; and Rust Indra text/math. Each Indra arm uses batch cap 64 and one worker, with explicit one-CPU or two-CPU thread settings. Tika 3.3.2, shared cgroup accounting, aggregate 3 GiB memory, zero swap, matching affinity, one request at a time, one warmup and three recorded repetitions remain fixed. The two CPU budgets are never pooled.

Pass this directory's `protocol.json`, `corpus-frozen.json` and `acquisition/tesseract-models.json` to the existing [Docker comparison driver](../runner/README.md). Point its corpus root at `lab-data` and the relevant Tesseract model root at `lab-data/tesseract/fast` or `lab-data/tesseract/best`; Indra uses the same immutable model snapshot as the parent study. The driver checks that `requests_per_repetition` is two for this manifest and that all selected arm settings and the exact Tika XML snapshot match the protocol.

Report each case's first-attempt NFC CER, whitespace-collapsed CER, WER, complete failures and paired timings at each CPU budget. Latency quantiles describe successful complete HTTP bodies across all three repetitions; report failure termination times separately and retain the finite sample counts. With only two article clusters, this supplement uses **descriptive comparisons without confidence intervals or population superiority claims**; the three repetitions are technical repeats. Retain failed and incomplete runs. No output-based case replacement, reference repair or best-performing configuration selection is allowed under this freeze.

### Prospective execution amendment

[Execution amendment 0001](audit/execution-amendment-0001.json) was recorded before any OCR inference on either supplemental input. It preserves the [original protocol bytes](audit/protocol-original-frozen.json), records the old and active protocol hashes, binds the finalized parent execution protocol and enumerates all nine changed JSON fields. The two cases, their references and language counts, all eight arm settings, resource budgets, repetitions, external request deadline and scoring rules are unchanged. The active protocol's original `parent_protocol_sha256` remains the scientific baseline identity; the amendment separately records the later parent execution identity.

The amendment adds exact hashes for the [common Python/Rust Indra XML](../../deploy/evaluation/tika-indra.xml) and [Tesseract XML](../../deploy/evaluation/tika-tesseract.xml). Both set the Tika OCR subprocess timeout to 60 seconds. Indra's internal deadline is 65 seconds, the bridge deadline is 70 seconds, the gateway deadline is 300 seconds and the external load generator deadline remains 120 seconds. A late native request remains accounted until cleanup even when the common Tika timeout has already ended its subprocess.

Startup means process and model initialization **after model inventory verification has warmed the host file cache**. It is measured separately from warmed HTTP requests and native profiling; it is not a storage-cold measurement. Each arm and CPU budget uses a new parent cgroup so `memory.peak` has a fresh lifetime. Parent memory counters report charged memory, including startup, rather than summed process RSS. File cache populated during acquisition can remain charged to the acquisition cgroup; the warmup-and-cohort counter delta cannot be treated as the complete model memory footprint. Image healthchecks and the optional gateway OCR readiness dependency are disabled uniformly during benchmark arms; the controller verifies both services before measurement.

[freeze.json](freeze.json) remains byte-identical and binds the original scientific inputs, source/model locks, references, audit and protocol. To verify that historical freeze, resolve its `protocol.json` entry to `audit/protocol-original-frozen.json`; every other listed path and byte remains unchanged. Use the amended `protocol.json` for execution and retain its amendment alongside the result. Any later audit or execution evidence should be a separate, hash-linked record preserving the original freeze, this amendment, and both failed and successful attempts.
