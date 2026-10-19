This controller implements the previously frozen
[seven-role plan](evidence/seven-role-plan-2026-10-18.json). Both build contexts
come from application source `2193324bf42984c9dd3009fd4226b96f2379fad3`.
They receive the same immutable SDK helper from `2a16bb3`. The candidate receives
only the Dockerfile and Cargo selector from `29695e8`; all other application,
model-lock, fixture, compiler, feature and profile inputs remain identical.

`prove-frozen-wp1.py prepare` captures complete context inventories, rejects any
undeclared source delta, reads and verifies the full frozen native SDK OCI input,
and renders mandatory SDK consumers with no native source-build fallback. The
controller/audit/plan bytes must match their recorded producing Git commit
before preparation. Its `run` action rechecks those exact bytes, the retained
baseline source archive, the prepared contexts and rendered recipes before any
builder observation. Later unrelated commits are permitted; changed audit
semantics, dirty producing tools or altered retained inputs fail admission. The
audit module is loaded only when needed, after its run-time identity check. The
controller then exports the original engine and server acceptance targets on one existing shared builder.
All four original Cargo commands remain in the immutable recipes; the candidate
adds JSON reporting and exact artifact selection.

After both builds, the controller requires exact SHA256 values for all seven
executables, static ELF checks, identical generated configuration/OpenAPI output,
and complete material checksum inventories. Each source archive independently
must contain every frozen-context file under the declared collector roots with
its exact bytes, excluding target directories and runtime/proofs. The model
lock cannot silently disappear from both arms. Generated .cargo/config.toml and
nonempty vendor sources are required and separately inventoried; .github and the
root README are not undeclared new collector operands. All native, glibc, vendor
and notice materials must match. Each of the 31 notices additionally binds the
verified SDK recipe source path, original materialized SDK bytes and inventory
component/destination/size/hash; a matching line count alone is insufficient. Both source archives are safely extracted and compared;
only the declared Dockerfile/selector source delta may differ. The differing
source archive and enclosing checksum file are retained and explained explicitly.

Run this only after the coordinator clears all active runtime measurements:

```sh
python3 scripts/prove-frozen-wp1.py prepare \
  --sdk-layout /absolute/path/to/frozen-sdk-oci \
  --output /absolute/path/to/new-wp1-proof
python3 scripts/prove-frozen-wp1.py run \
  --output /absolute/path/to/new-wp1-proof
```

The SDK must be the frozen `sha256:3a98d2e4588bfb13fbca7caac07ba8caa060d4af07a362f5efa21557a4cbe14a`
platform manifest with recipe `d532e54f547ca606d2df1d546fb3ef94c372160efda4bbd12877435b7d0e8bc7`
and content `b4810a9984dcb21493cc330e2f08b28849429d5c3cb055cff29e0733356b5411`.
This is a content comparison on a shared builder. Phase wall time and direct
client accounting are diagnostic; they establish no controlled speedup, compiler
CPU quota, fresh-runner restoration, signing or lifecycle/deployment acceptance.
Those gates remain separate. Preparation and execution have not run yet.
