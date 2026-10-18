# Kubernetes version receipt correction

The frozen 2193324 release failed in [run 39166554904, job 117790900430](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39166554904/job/117790900430), after creating the owned kind cluster, at `crictl version -o json` (`flag provided but not defined: -o`). That failed candidate remains failed; this correction does not supply a deployment pass or authorize signing it.

The [kind v0.33.0 base Dockerfile](https://github.com/kubernetes-sigs/kind/blob/v0.33.0/images/base/Dockerfile) pins cri-tools v1.36.0. Its [`version` command](https://github.com/kubernetes-sigs/cri-tools/blob/v1.36.0/cmd/crictl/version.go) defines no output flag and prints four runtime fields as plaintext. The global `--version` reports the client separately.

`record-cri-version.sh` invokes both supported forms with finite deadlines, retains separate raw stdout/stderr, validates the pinned client and the four runtime fields, and constructs `cri-version/result.json`. Its `parsed-crictl-plaintext` marker identifies the JSON as our receipt rather than native crictl output. Image identity, Kubernetes version, cgroup admission, OCR and lifecycle acceptance gates remain required.

`tests/cri-version.sh` exercises the exact command arguments without Docker, including rejection of the original unsupported flag, missing/duplicate/extra runtime fields, a failed runtime query, a mismatched client version, and output overwrite. Both deployment acceptance and release preflight run it before expensive builds. Initial source preparation occurred during an active benchmark hold; test execution and a new full Kubernetes acceptance run must be recorded separately.
