# Retained perf event-filter correction

The failed workflow recorded valid `cpu-clock:u` events but passed that entire
string to Inferno 0.12.8. Its parser splits event fields at the first colon and
compares `cpu-clock`, so the old filter produced no folded stacks.

Reprocessing the retained raw script with `--event-filter cpu-clock` admits
432 samples, 432 folded weights and zero lost samples. The raw-stream validator
still requires `cpu-clock:u` and the exact OCR process ID. Of those samples,
59 contain unknown frames. This is diagnostic recovery from the original hosted
run, not new local-host execution or a complete profiling result.

`receipt.json` binds the retained inputs and corrected output. The complete
one-/two-CPU TIFF/math experiment and reviewed visualizations remain required.
