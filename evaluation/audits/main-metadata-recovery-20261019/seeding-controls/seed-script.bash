set -euo pipefail
AUDIT="$CASE_AUDIT"
OCR_CORPUS="$CASE_CORPUS"
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
