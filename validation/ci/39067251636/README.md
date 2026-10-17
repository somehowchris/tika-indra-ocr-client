# Hosted baseline: completed verification, blocked release

[Run 39067251636](https://github.com/somehowchris/tika-indra-ocr-client-/actions/runs/39067251636)
tested commit `bfb038788937f307dcf0666aa0da9b7954042b7c` on 17 October 2026.
The reusable verification job passed. The publication job failed before signing
and before its timed container benchmark. This run does **not** establish a
successful signed release or signed-image performance results.

## What executed

| Gate | Result |
| --- | --- |
| Rust correctness, static GNU build and protocol tests | Passed |
| Real backend HTTP/input tests; worker recovery and log handling | 12 and 5 tests passed |
| Local Docker builds and Compose Indra/Tesseract checks at 1 and 2 total CPUs | Passed |
| Kubernetes deployment, OCR correctness and admitted/kernel resource checks at 1 and 2 total CPUs | Passed |
| Four GHCR builds and publication | Passed |
| Exact published image pulls and Compose Indra/Tesseract checks at both CPU budgets | Passed |
| Kubernetes check of published candidates | Failed during image archive validation, before cluster creation |
| Container signing, SBOM attestation and signature verification | Skipped after failed gate |
| Timed signed-container benchmark | Skipped after failed gate |

`run.json` retains exact job/step status and artifact metadata. The two full job
logs have local SHA-256 records in `logs.metadata.json`. Each artifact metadata
file contains GitHub's source commit, artifact ID, byte count and SHA-256. ZIP
digests, lengths and CRCs were checked before safe extraction. Raw Docker build
records are retained outside git; their metadata records exact source and hashes.

## Concrete release defect

Docker 29.9.1 exported one runnable image **and its attached OCI provenance
artifact** with `docker image save --platform linux/amd64`. The previous verifier
required exactly one entry in `index.json` and rejected the legitimate artifact:
`Expected exactly one exported platform-specific image`.

The pinned [Docker exporter](https://github.com/moby/moby/blob/b26fb77d3e7b01f9d21b89539c1ab428f5ca431f/daemon/containerd/image_exporter.go)
registers a referrers provider. Its [containerd exporter](https://github.com/moby/moby/blob/b26fb77d3e7b01f9d21b89539c1ab428f5ca431f/vendor/github.com/containerd/containerd/v2/core/images/archive/exporter.go)
includes those descriptors in the archive index. The
[subject annotation](https://github.com/moby/moby/blob/b26fb77d3e7b01f9d21b89539c1ab428f5ca431f/vendor/github.com/containerd/containerd/v2/core/images/annotations.go)
distinguishes referrers from image names. `source-evidence.json` records immutable
source URLs, Git blob IDs and content SHA-256. Exact published Tika manifest reads
confirmed this subject binding; `published-tika-manifests.json` records the parsed
responses and the digest-verification method.

The corrected gate preserves the selected manifest/config identity checks. It
accepts additional entries only when the hashed OCI artifact and the containerd
annotation both name the exact selected manifest. Artifact config must be empty
and non-runnable; its payloads must be present and hash-valid. Extra runnable
images, mismatched subjects, and missing or corrupt provenance remain errors.

## Local reproduction of the correction

`oci-provenance-reproduction.json` records Docker 29.9.1 pulling the exact
published scratch bridge image, selecting amd64 and exporting it with attached
provenance. The old one-entry condition fails on that real export. The corrected
gate passed, retaining the original manifest, configuration and provenance
digests. Thirteen focused positive/negative tests passed; `archive-tests.json`
records the commands, results and tested source hashes.

This reproduction checks real image transport and archive identity. A subsequent
hosted run must still complete Kubernetes, signing and timed benchmarking for
the corrected source. Historical run failures and successful measurements remain
separate evidence.
