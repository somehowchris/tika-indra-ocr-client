# Synthetic math decoder fixture

`math.png` is the fixed grayscale rendering of `math.tex`. It is an implementation
fixture, excluded from the public quality corpus and representative end-to-end
performance comparisons. Its native diagnostic profiles are analyzed separately.
The native unit gate passes 64 deliberate copies of the first polynomial's
rectangle directly to the actual math reader at batch caps 1, 8 and 64. It checks
every accepted line against the declared LaTeX and observes the actual encoder
input shapes. Orientation, detection and the classifier have separate full-page
acceptance cases.

The locked Indra 0.3.2 reference produced the same equation for every row at all
three caps before the native test was written. Its actual encoder histograms
were `B1 × 64`, `B8 × 8` and `B64 × 1`; decoder calls were 1,472, 184 and 23.
Captured and uncaptured reference readings matched. These repeated rows are not
independent quality samples, and these call counts are not speed measurements.
The math models and `math_meta.json` are byte-identical in the v1.1.0 snapshot,
and the declared equation reads the same under Indra 1.1.2.

Reference implementation: [Indra commit
0e96b93c6810f17cf8352acbb4487cc9e45b66d5](https://github.com/datalab-to/indra/blob/0e96b93c6810f17cf8352acbb4487cc9e45b66d5/indra_ocr/math.py)
(indra-ocr 1.1.2), FP32 models from snapshot
`bd02167fdf2005a19962cc15188f8496a2c87d3b` (tag v1.1.0), CPython 3.13.15,
NumPy 2.5.4, ONNX Runtime 1.31.0 and OpenCV 5.0.0.93; ORT/OpenCV/helper thread
settings were all one. Native acceptance must still pass against the actual
linked build.

The fixture was rendered with pdfTeX 1.40.25 / TeX Live 2023 and Poppler:

```sh
pdflatex -interaction=nonstopmode -halt-on-error math.tex
pdftoppm -png -gray -r 200 -singlefile math.pdf math
```

The checked-in raster is the test input; rerendering with different fonts or
tool versions can change its bytes.

| File | SHA-256 |
|---|---|
| `math.png` | `5482d2f8acdedda736c020bfde851a5358730dad168e2ad7920a38f21608ab61` |
| `math.tex` | `3e920a190de52e0636cebbcae244dda443d872a85f8eed24d6367e87e989afcf` |

## Frozen profiling output bytes

`two-pages.expected.txt` and `math-page.expected.txt` are the exact outputs of
the unmodified indra-ocr 1.1.2 reference on the v1.1.0 snapshot (math enabled
for the math page), produced on 2026-10-20 with the environment above. They are
not human-transcribed accuracy gold. The two-page TIFF text (33 bytes) is
unchanged from the 0.3.2 reference; the math page changed with the retrained
line models (281 bytes, SHA-256
`babb4026e2619b03288abe3f5486cf56b4e4f6c7c18c92ad8ac0c0c87635eb5a`). The 0.3.2
bytes (293 bytes, `b712aafe…`) remain in the 2026-10-18 parity evidence. The
profiling protocol locks these bytes separately from the quality corpus.
