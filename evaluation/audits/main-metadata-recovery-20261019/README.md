# Approved main corpus metadata recovery

The original [119-entry source lock](../../acquisition/frozen-downloads.json) stays unchanged. Its earlier fetch verified 97 non-metadata sources; default fetch skips metadata. This separate acquisition amendment resolves the 22 missing metadata records without changing any scored input/reference, model, cohort, metric, protocol or runtime/default matrix behavior.

[amendment.json](amendment.json) records root approval of the unchanged [original proposal](original-proposal.json), SHA-256 75876429901a99902ff8ffc288210ecc8d8510a572bb0b48d8c13ce0f28801ee. The sealed recovery packet retains its historical pending proposal; approval is this separate record.

| Recovery | Records | Retained evidence |
|---|---:|---|
| Raw HTTP bytes match original size/hash | 12 | Actual responses, redacted headers and commands |
| Exact original file contents reconstructed from GitHub JSON | 4 | Compact JSON only; three ASCII-escaped and Spanish UTF-8; original size/hash match while raw HTTP mismatch remains |
| Original ancillary metadata unavailable | 6 | Changed BHL/UCI HTTP200 bodies and four empty EUR-Lex HTTP202 challenge responses; none admitted |

Current BHL primary metadata and its pinned original README support CC-BY-3.0. Current UCI HTML states CC-BY-4.0 and the same DOI; its API body recovered exactly. EUR-Lex article HTML and its legal notice returned empty HTTP202 responses. Existing qualified PDF/reference/rights audits remain separate historical evidence. This recovery establishes neither renewed EUR-Lex rights verification nor literal gold, unseen training data or representative legal/clinical accuracy.

## Portable evidence

First run:

    sha256sum -c SHA256SUMS

- recovery-proof.tar.zst contains the complete unchanged sealed recovery packet: all 22 attempts, two supporting observations, both pinned hf XML checks, raw/derived hashes, controls, earlier controller/analysis attempts and cleanup. Its inner seal hash is 2f69bf55d8b5f8ff889ba5f66e7709e3c64799b82538686544fe0542e4facceb.
- exact-metadata.tar.zst contains only the 16 exact original metadata files. exact-metadata.tsv supplies original sizes, hashes and paths; exact-metadata.sha256 is directly checkable.
- archives.json records actual tar/zstd commands and complete streaming path/type/mode/size/hash verification. No executable, model, credential or CA material is included.
- metadata-audit-sources.json retains 113 unchanged original rows in order: 97 data sources and 16 recovered metadata records. index-derivation.json binds both indexes and the six exclusions. It is a separate acquisition audit, not a replacement frozen study lock.

## Seed without overwriting

Use Bash and GNU tar/coreutils. Set OCR_CORPUS to the existing root prepared through the [normal acquisition route](../../runner/README.md#build-and-prepare). An existing different file or symlink fails.

~~~bash
set -euo pipefail
AUDIT="$PWD/evaluation/audits/main-metadata-recovery-20261019"
OCR_CORPUS="/absolute/path/to/corpus"
(cd "$AUDIT" && sha256sum -c SHA256SUMS)
test -d "$OCR_CORPUS"
test ! -L "$OCR_CORPUS"
test ! -L "$OCR_CORPUS/metadata"
mkdir -p "$OCR_CORPUS/metadata"
METADATA_SEED="$(mktemp -d "$OCR_CORPUS/.metadata-seed.XXXXXXXX")"
trap 'rm -rf -- "$METADATA_SEED"' EXIT
tar --zstd --extract --file "$AUDIT/exact-metadata.tar.zst" \
  --directory "$METADATA_SEED" --keep-old-files \
  --no-same-owner --no-same-permissions
(cd "$METADATA_SEED" && sha256sum -c "$AUDIT/exact-metadata.sha256")
while IFS=$'\t' read -r expected_bytes expected_hash relative; do
  test "$(wc -c < "$METADATA_SEED/$relative")" -eq "$expected_bytes"
  test ! -L "$OCR_CORPUS/$relative"
  if test -e "$OCR_CORPUS/$relative"; then
    test -f "$OCR_CORPUS/$relative"
    cmp -- "$METADATA_SEED/$relative" "$OCR_CORPUS/$relative"
  else
    ln -- "$METADATA_SEED/$relative" "$OCR_CORPUS/$relative"
  fi
done < "$AUDIT/exact-metadata.tsv"
(cd "$OCR_CORPUS" && sha256sum -c "$AUDIT/exact-metadata.sha256")
~~~

The temporary is on the corpus filesystem, so hardlink publication is atomic and never overwrites. Its exit trap removes only the owned temporary.

## Actual offline verification

The unchanged source41 static GNU evaluator, source 60c849a0f829c7813c5e3fbfb2dc6e907eda1958 and SHA-256 2b69a6f930dfcafae57d184d286e204ea64ee775f391b74d1d8e0c40cf3de70c, ran:

    /indra-evaluate fetch --include-metadata --lock /locks/sources.json --root /corpus

All 113 ordered rows verified with networking disabled and the existing corpus read-only. The [actual result](offline-verification/output/result.json), complete stdout/stderr, source41 Cargo/static identities, Docker records, kernel counters and cleanup are retained under offline-verification/.

The existing GNU helper was image 56e865745a613045b95c3f9e478765dd2515baba4f611038b58179da1ce64239. Actual controls were 2 CPUs, affinity 12,13, 512 MiB memory, zero swap, 128 PIDs, read-only root/inputs, dropped capabilities and no new privileges. Kernel checks and owned-container removal passed; OOM counters remained zero. This proves the 113-row audit inventory, not 119 exact records or OCR speed/quality.

For replay, the exact source41 evaluator and recorded GNU image are separate inputs; their bytes are not embedded in these metadata archives. The complete actual controllers and argv are retained. Use new output paths. A simple offline cache check is:

~~~bash
OCR_EVALUATOR="/absolute/path/to/source41/indra-evaluate"
printf '%s  %s\n' \
  2b69a6f930dfcafae57d184d286e204ea64ee775f391b74d1d8e0c40cf3de70c \
  "$OCR_EVALUATOR" | sha256sum -c -
docker run --rm --network none --read-only --cap-drop ALL \
  --security-opt no-new-privileges:true --cpus 2 --cpuset-cpus 12,13 \
  --memory 512m --memory-swap 512m --pids-limit 128 \
  --user "$(id -u):$(id -g)" \
  --mount "type=bind,src=$OCR_CORPUS,dst=/corpus,readonly" \
  --mount "type=bind,src=$AUDIT/metadata-audit-sources.json,dst=/locks/sources.json,readonly" \
  --mount "type=bind,src=$OCR_EVALUATOR,dst=/indra-evaluate,readonly" \
  --entrypoint /indra-evaluate \
  sha256:56e865745a613045b95c3f9e478765dd2515baba4f611038b58179da1ce64239 \
  fetch --include-metadata --lock /locks/sources.json --root /corpus
~~~

Acquisition affinity does not change the frozen scientific CPU profiles. Historical 97-source and 82 main plus 4 laboratory prepared-file receipts retain their original producers. No OCR, model loading, compilation or scientific timing occurred.
