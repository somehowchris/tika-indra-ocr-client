# Kubernetes version receipt correction

The frozen 2193324 release failed in [run 39166554904, job 117790900430](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39166554904/job/117790900430), after creating the owned kind cluster, at `crictl version -o json` (`flag provided but not defined: -o`). That failed candidate remains failed; this correction does not supply a deployment pass or authorize signing it.

The [kind v0.33.0 base Dockerfile](https://github.com/kubernetes-sigs/kind/blob/v0.33.0/images/base/Dockerfile) pins cri-tools v1.36.0. Its [`version` command](https://github.com/kubernetes-sigs/cri-tools/blob/v1.36.0/cmd/crictl/version.go) defines no output flag and prints four runtime fields as plaintext. The global `--version` reports the client separately.

`record-cri-version.sh` invokes both supported forms with finite deadlines, retains separate raw stdout/stderr, validates the pinned client and the four runtime fields, and constructs `cri-version/result.json`. Its `parsed-crictl-plaintext` marker identifies the JSON as our receipt rather than native crictl output. Image identity, Kubernetes version, cgroup admission, OCR and lifecycle acceptance gates remain required.

`tests/cri-version.sh` exercises the exact command arguments without Docker, including rejection of the original unsupported flag, missing/duplicate/extra runtime fields, a failed runtime query, a mismatched client version, and output overwrite. Both deployment acceptance and release preflight run it before expensive builds. Initial source preparation occurred during an active benchmark hold; test execution and a new full Kubernetes acceptance run must be recorded separately.

## Actual pinned client output, 19 October

[Run 39213042394, publish job 117931205618](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39213042394/job/117931205618), source `5ae8b82`, passed Compose but failed the corrected preflight parser with exit 4. The owned kind control plane became Ready; application image import and Kubernetes OCR/lifecycle admission had not started. The retained 22-byte client stream is `crictl version 1.36.0\n`, while the first parser expected `v1.36.0`. The runtime stream reports containerd `v2.3.4` and API `v1`. Later reference, trace, signing and SDK/Chef publication were skipped.

The immutable [cri-tools Makefile](https://github.com/kubernetes-sigs/cri-tools/blob/88d8ad9d40f82726fda53c2d271e6172b4c619c9/Makefile#L37) strips the leading tag prefix before linking the CLI version. The [unchanged captured fixtures](../../tests/fixtures/cri-version-1.36.0/README.md) now drive the positive regression. The recorder accepts exactly numeric `1.36.0`; wrong versions, a leading `v`, suffixes, malformed runtime fields, command failures and overwrite still fail. Parsing errors name the expected contract and retained raw streams while preserving the original jq status.

Independent offline replay reproduces the old exit 4 and the fixed exit 0 on the same captured bytes. The main Kubernetes gate remains byte-identical. These checks establish the parser correction; a focused remote replay and a complete current-source release remain required for deployment/signing acceptance.
