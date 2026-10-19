# Native text and Tika's XHTML boundary

The native HTTP API preserves Indra's `\n\f\n` separator between image frames.
The Tesseract-compatible bridge omits only the form-feed byte (`U+000C`), honoring
the supported empty `page_separator=` setting when saving Tika's `.txt` result.
All other UTF-8 bytes, including
whitespace, punctuation and literal replacement characters, remain unchanged.

Tika 3.3.2's [OCR output reader](https://github.com/apache/tika/blob/3.3.2/tika-parsers/tika-parsers-standard/tika-parsers-standard-modules/tika-parser-ocr-module/src/main/java/org/apache/tika/parser/ocr/TesseractOCRParser.java)
passes the text through an XHTML handler. Its
[SafeContentHandler](https://github.com/apache/tika/blob/3.3.2/tika-core/src/main/java/org/apache/tika/sax/SafeContentHandler.java)
replaces XML-invalid characters; form feed is excluded by XML 1.0's character
production. Without the adapter, each native page separator becomes `U+FFFD` in
Tika's final text. Omitting this framing control preserves the surrounding line
endings and keeps the page texts separated without adding a visible character.

The adapter forwards borrowed valid spans and consumes only the page marker.
It performs no allocation, whole-response copy or Unicode normalization. Its
`Write` implementation preserves partial writes and errors. The bridge continues
to validate UTF-8 and the declared HTTP length and removes its own partial file
if saving fails. The generic `stream_utf8` reader remains byte-preserving.

## Reproduced failure and regression

[Deployment run 39149717827](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39149717827)
tested PR merge `b818d5953d8f2bc736309168be51d7a96587298d`, from PR head
`09120a115c2636b3420864dc2354ce8d8e2ef79d`. It passed native image builds,
gateway process gates, OCI root/platform/config binding and actual Tika/native
readiness. The first real two-frame TIFF returned
`INVOICE 12345\n�\nSECOND PAGE 67890\n\n`. Its exact-content assertion failed
before the admitted-work shutdown experiment. Kubernetes was skipped.

The complete retained artifact is `12145221292`, 99,708,898 bytes, SHA-256
`001f57cd0d48e027a5c539cfd808010d16be0052272b7d1e36438aa663fe74a3`.
The boundary regression covers the exact native body, leading and consecutive
form feeds, multibyte UTF-8 across reader boundaries and writer failure. The
4 MiB allocator gate includes form feeds at the beginning, a buffer boundary and
the end; all six allocator counters must be zero. Whole-stack acceptance still
requires a fresh real Docker/Kubernetes run. The harness must not accept extra
replacement characters to hide this integration defect.
