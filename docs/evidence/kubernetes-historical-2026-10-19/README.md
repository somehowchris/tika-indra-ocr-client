# Historical Kubernetes acceptance

[Run 39230388131](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39230388131) passed both CPU phases using the original source-`5ae8b82` images and harness. Controller `2496534` changes only admitted diagnostic orchestration and the reviewed CRI, Docker-owner-cgroup and raw server-PodList guards. The earlier failures remain in runs [39213042394](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39213042394), [39225961004](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39225961004) and [39228810620](https://github.com/somehowchris/tika-indra-ocr-client/actions/runs/39228810620).

The [upstream termination contract](https://kubernetes.io/docs/concepts/workloads/pods/sidecar-containers/#sidecar-containers-and-pod-lifecycle) keeps restartable sidecars alive until the main container stops, subject to the Pod termination grace period. The retained lifecycle records prove actual Tika 3.3.2 OCR before shutdown, removal of readiness, rejection of new requests, byte-identical completion of accepted work and command exit zero. Exact original Pod UID/container IDs have clean terminal watch evidence after CRI garbage collection. Both phases had zero observed OOM events and exact cleanup. Internal Java termination by SIGTERM remains compatible with the supervisors' successful exit; this is no claim that no child receives a signal.

The whole kind node has numeric 1/2-CPU limits and affinity `0` / `0-1`. The native/Tika containers have 0.75/0.25 CPU at one CPU and 1.5/0.5 at two CPUs, 2/1 GiB memory and zero swap. The node's nested process leaf is unlimited; its exact Docker-owned ancestor supplies the binding CPU quota. Other ancestors are retained observations. Control-plane memory is not capped by this experiment. Readiness and shutdown durations are functional observations, not performance benchmarks.

`raw/` contains 23 unmodified members checked byte-for-byte against the complete original ZIP. `summary.json` records complete-archive verification; the two independent reviews were produced before that download completed and their original pending status is preserved in `reviewed-host-records.json`. This small packet supports inspection and is not a replacement for the full watch, OCI, resource, source and log evidence.

Download the original authenticated GitHub artifact:

```sh
gh run download 39230388131 --repo somehowchris/tika-indra-ocr-client \
  --name kubernetes-replay-39213042394-2496534eb248cd1286c5373013fc38646cd6c67b-39230388131-1
```

The exact artifact ZIP (ID `12176178949`) is 826,503,205 bytes, SHA-256 `5beb7b3581131eb836e413ea00d2ec4782469faf995a27eeb454a66eb137f34b`. The GitHub CLI extracts downloads; obtain the original ZIP through the artifact API to check that transport hash. Every one of 900 ranged member records and 324 selected member comparisons matched the verified full archive.

This is historical functional acceptance. It establishes no source-`60c849a` signature, SDK restoration, representative OCR accuracy or candidate speedup.
