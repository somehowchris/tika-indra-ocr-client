#!/usr/bin/env bash
# Reproducible reporting only. No OCR or benchmark execution.
set -Eeuo pipefail
report=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
command -v uv >/dev/null
command -v jq >/dev/null
# tools.json is the single record of the environment that rendered the report.
[[ $(uv --version | cut -d ' ' -f 2) == "$(jq -er '.uv.version' "$report/tools.json")" ]] || { printf 'uv must be the pinned version in tools.json\n' >&2; exit 1; }
uv venv --allow-existing --python "$(jq -er '.python' "$report/tools.json")" "$report/.venv"
uv pip sync --python "$report/.venv/bin/python" --require-hashes "$report/requirements.lock"
mkdir -p "$report/.tools" "$report/render-evidence"
archive=$report/.tools/quarto-1.10.19-linux-amd64.tar.gz
quarto=$report/.tools/quarto-1.10.19/bin/quarto
if [[ ! -f $archive ]]; then
  curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 \
    --connect-timeout 15 --max-time 180 \
    https://github.com/quarto-dev/quarto-cli/releases/download/v1.10.19/quarto-1.10.19-linux-amd64.tar.gz \
    --output "$archive.partial"
  printf '%s  %s\n' 8e38c2b2df6a6dbd369ea16e15acded6894545258d13a02f777f81ad18fd7973 "$archive.partial" | sha256sum --check --status
  mv -- "$archive.partial" "$archive"
fi
printf '%s  %s\n' 8e38c2b2df6a6dbd369ea16e15acded6894545258d13a02f777f81ad18fd7973 "$archive" | sha256sum --check --status
tar -xf "$archive" -C "$report/.tools"
export QUARTO_PYTHON=$report/.venv/bin/python
export MPLBACKEND=Agg
export PYTHONDONTWRITEBYTECODE=1
"$QUARTO_PYTHON" -m unittest discover -s "$report" -p 'test_*.py' -v 2>&1 | tee "$report/render-evidence/tests.log"
"$QUARTO_PYTHON" -O -m unittest discover -s "$report" -p 'test_*.py' -v 2>&1 | tee "$report/render-evidence/tests-optimized.log"
# Draw every study figure once from labelled synthetic rows so the Seaborn
# artist checks run before the real render; the output never enters the report.
smoke=$report/render-evidence/synthetic-smoke
rm -rf -- "$smoke"
OCR_REPORT_SYNTHETIC_SMOKE=$smoke "$QUARTO_PYTHON" -B -m unittest discover -s "$report" -p 'test_figures.py' -v 2>&1 | tee "$report/render-evidence/synthetic-smoke.log"
[[ $(find "$smoke" -maxdepth 1 -name '*.png' | wc -l) -eq 6 ]]
"$quarto" render "$report/report.qmd" --to html 2>&1 | tee "$report/render-evidence/render.log"
"$QUARTO_PYTHON" -O -c 'import sys; sys.path.insert(0, sys.argv[1]); import baseline; baseline.load()' "$report"
"$QUARTO_PYTHON" "$report/receipt.py"
