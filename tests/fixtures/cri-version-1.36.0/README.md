These are the unchanged client/runtime stdout bytes captured from the pinned
kind node in [release run 39213042394](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39213042394),
failed publish job 117931205618, artifact 12172057374. Producing application source:
`5ae8b82f65a6c9aafbaa46554d2ece9137bea40d`. The 22-byte client reply contains
`crictl version 1.36.0`; the prior parser required a leading `v` and exited 4
before importing application images or running Kubernetes lifecycle acceptance.

The pinned [cri-tools Makefile](https://github.com/kubernetes-sigs/cri-tools/blob/88d8ad9d40f82726fda53c2d271e6172b4c619c9/Makefile#L37)
strips the tag prefix before injecting the CLI version. These fixtures establish
the parser regression; they do not establish an OCR or Kubernetes shutdown result.
