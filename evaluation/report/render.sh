#!/usr/bin/env bash
# Reproducible reporting only. No OCR or benchmark execution.
set -Eeuo pipefail
report=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)
command -v uv >/dev/null
uv venv --allow-existing --python 3.12.11 "$report/.venv"
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
"$quarto" render "$report/report.qmd" --to html 2>&1 | tee "$report/render-evidence/render.log"
"$QUARTO_PYTHON" -O -c 'import sys; sys.path.insert(0, sys.argv[1]); import baseline; baseline.load()' "$report"
"$QUARTO_PYTHON" "$report/receipt.py"
