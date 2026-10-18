# Synthetic math decoder fixture

`math.png` is the fixed grayscale rendering of `math.tex`. It is an implementation
fixture, excluded from the public quality corpus and performance estimates.
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

Reference implementation: [Indra commit
3d773aeedc5d79d1d0a2a04007a9075cd71a59f4](https://github.com/datalab-to/indra/blob/3d773aeedc5d79d1d0a2a04007a9075cd71a59f4/indra_ocr/math.py),
FP32 models from snapshot `ed345c7f158eddbc7dee0c6a7269afb75a94f7e8`,
CPython 3.13.16, NumPy 2.5.4, ONNX Runtime 1.31.0 and OpenCV 5.0.0.93;
ORT/OpenCV/helper thread settings were all one. Native acceptance must still
pass against the actual linked build.

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
