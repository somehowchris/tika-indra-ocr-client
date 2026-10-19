# Diagnostic Compose lifecycle receipt

Both real 1/2-CPU lifecycle gates passed with the exact images recorded here.
This mixed-build diagnostic is **not acceptance of signed source 2193324** and
is not a throughput benchmark. The controller is source 3513a06; unmodified
container identity checks, strict fixture text checks and actual kernel quotas
all ran. Readiness disappeared, new work was rejected, accepted work completed
with byte-identical output, and all service containers exited zero without OOM
or restart. All owned containers, networks and model-cache volume were removed.

The first attempt failed because the minimal diagnostic image omitted /models.
The diagnostic derivative adds only an empty UID/GID-65532-owned directory;
its native executable is byte-identical to the canonical base. Production's
Dockerfile already includes this owned directory. That failure remains visible.

`receipt.json` records provenance limits, raw evidence paths and inventory hash.
The selected files have their own `SHA256SUMS`; multi-megabyte OCI archives are
excluded. Kubernetes was not executed locally: creating the required kind node
would introduce a privileged container. The separate release CI gate remains
required. No successful Kubernetes result is inferred from these Compose runs.
