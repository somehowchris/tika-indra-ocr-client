# Private profiling bundle transport

The host profiling bundle stays owned by its existing user with its existing
0700 root directory. The outer Docker controller verifies the full original
bundle and source closure on the host, then streams tar through stdin into a
unique controller-owned ordinary local Docker volume. No root container mounts
or changes the host bundle. Extraction uses `--no-same-owner`: only the copied
files become owned by container root. Source file/directory modes stay exact.

The seed container has no capabilities, no network, a read-only root filesystem,
one CPU, 3GiB memory, zero swap and a finite lifetime. Only its owned volume is
writable. It verifies both complete original hash manifests and emits a complete
file/directory/type/mode inventory for exact comparison with the host. Each
profiling phase admits that volume's owner label, local driver and null options,
then mounts it read-only. Original CPU/resource barriers, numerical gates,
profiling protocol and inner script remain unchanged.

When the outer controller changes independently of the compiled baseline, point
`PROFILE_SOURCE_ROOT` to the clean frozen source tree named by the bundle. The
inner source admission still requires its exact checked-out commit and complete
source bytes. Three execution-source bind mounts come from that tree; the new
outer controller and volume-admission filter receive separate identity hashes.

Run the actual numerical control gate before any timing:

```sh
PROFILE_SOURCE_ROOT=/vol/md1/tika-indra-benchmark \
  bash /vol/md1/tika-indra-private-profile/scripts/profile-native-docker.sh \
  /vol/md1/tika-indra-artifacts/profile-2193324 \
  /vol/md1/tika-indra-artifacts/models \
  /absolute/new-private-control-evidence \
  sha256:1d14bce8a84a4fb4bb251e72f1897d693bba5839a0a424a12a30ac96e383e142 \
  --gates-only
```

This runs only the original gates at one and two CPUs. It never runs Criterion
or perf sampling. After review, omit `--gates-only` for the original four-phase
protocol. Keep other builds and measurements stopped during that study.

Every Docker operation and seed/phase process has a finite deadline. Cleanup
stops/removes only the exact token/image-owned container and removes only the
same token-owned local volume with null options. It audits volume absence and
reports failed ownership or cleanup as failure. Host source permissions are
recorded before/after; original raw evidence remains on the host after the
private working volume is removed.
