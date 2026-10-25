#!/usr/bin/env bash
# Candidate correctness/material gates. Scientific measurements run separately
# against the signed index; these smoke/process checks make no speedup claim.
set -euo pipefail
export LC_ALL=C
fail() { printf '%s\n' "$*" >&2; exit 1; }
[[ $# = 2 ]] || fail 'usage: verify-release-images.sh prepare|process|references|index EVIDENCE_DIRECTORY'
mode=$1
evidence=$(realpath -- "$2")
script_directory=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
[[ -d "$evidence/build" && ${GITHUB_SHA:-} =~ ^[a-f0-9]{40}$ ]] || fail 'build evidence and exact source identity are required'
bounded() { local limit=$1; shift; timeout --signal=TERM --kill-after=10s "$limit" "$@"; }
declare -A images image_ids acceptance_ids
for role in native gateway bridge tika tesseract backend; do
  images[$role]=$(jq -er --arg role "$role" 'select(.role == $role) | .reference' "$evidence/build/$role/image.json")
  [[ ${images[$role]} =~ ^ghcr\.io/[a-z0-9._-]+/tika-indra-$role@sha256:[a-f0-9]{64}$ ]] || fail 'invalid candidate image record'
done
container=
compose_active=0
compose=()
cleanup() {
  local status=$?
  trap - EXIT
  if [[ -n "$container" ]]; then bounded 40s docker rm --force "$container" > "$evidence/material-container-cleanup.log" 2>&1 || status=1; fi
  if (( compose_active )); then
    bounded 30s "${compose[@]}" logs --no-color > "$evidence/reference-cleanup-containers.log" 2>&1 || true
    bounded 600s "${compose[@]}" down --remove-orphans --volumes > "$evidence/reference-cleanup.log" 2>&1 || status=1
  fi
  exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

load_ids() {
  for role in native gateway bridge tika tesseract backend; do
    image_ids[$role]=$(jq -er 'select(.passed == true and .store == "containerd" and
      .image_id == .root_digest) | .root_digest' "$evidence/images/$role-resolution/result.json")
    [[ ${image_ids[$role]} =~ ^sha256:[a-f0-9]{64}$ ]] || fail 'missing immutable local image ID'
  done
}
load_acceptance_id() {
  local role=$1
  acceptance_ids[$role]=$(jq -er 'select(.passed == true and .store == "containerd" and
    .image_id == .root_digest and .architecture == "amd64" and .os == "linux") | .root_digest' \
    "$evidence/acceptance-build/$role/resolution/result.json")
  [[ ${acceptance_ids[$role]} =~ ^sha256:[a-f0-9]{64}$ ]] || fail 'missing immutable acceptance image ID'
}
bind_container() {
  local resolution=$1 inspection=$2 destination=$3
  bounded 950s bash "$script_directory/verify-daemon-image.sh" bind "$resolution" "$inspection" "$destination"
}
create_material_container() {
  local expected=$1 resolution=$2 prefix=$3
  [[ -z "$container" ]] || fail 'material container is already owned'
  [[ "$expected" =~ ^sha256:[a-f0-9]{64}$ ]] || fail 'material extraction requires an immutable image ID'
  container=$(bounded 40s docker create --platform linux/amd64 "$expected")
  [[ "$container" =~ ^[a-f0-9]{64}$ ]] || fail 'Docker did not return an exact container ID'
  bounded 30s docker inspect "$container" > "$prefix-container.json"
  bind_container "$resolution" "$prefix-container.json" "$prefix-binding"
}
remove_material_container() {
  bounded 40s docker rm "$container" > /dev/null
  container=
}
static_binary() {
  local binary=$1
  bounded 30s readelf -W -l "$binary" > "$binary.program-headers.txt"
  bounded 30s readelf -W -d "$binary" > "$binary.dynamic.txt"
  if grep -q INTERP "$binary.program-headers.txt"; then fail 'Rust executable has a program interpreter'; fi
  if grep -q NEEDED "$binary.dynamic.txt"; then fail 'Rust executable has a shared-library dependency'; fi
  sha256sum "$binary" > "$binary.sha256"
}

prepare() {
  [[ ! -e "$evidence/images" && ! -e "$evidence/materials" ]] || fail 'refusing to overwrite candidate evidence'
  mkdir "$evidence/images" "$evidence/materials" "$evidence/bin" "$evidence/acceptance-build"
  for role in native gateway bridge tika tesseract backend; do
    bounded 600s docker pull --platform linux/amd64 "${images[$role]}" > "$evidence/images/$role-pull.log" 2>&1
    dockerfile=$(jq -er .dockerfile "$evidence/build/$role/image.json")
    target=$(jq -r .target "$evidence/build/$role/image.json")
    bounded 600s bash "$script_directory/verify-image-provenance.sh" "${images[$role]}" "$dockerfile" "$target" "$evidence/images/$role-provenance"
    bounded 950s bash "$script_directory/verify-daemon-image.sh" resolve "${images[$role]}" "$evidence/images/$role-resolution"
    jq -e --arg reference "${images[$role]}" --slurpfile registry "$evidence/images/$role-provenance/result.json" '
      .passed == true and .store == "containerd" and .image_id == .root_digest and
      .root_digest == ($reference | split("@")[1]) and $registry[0].passed == true and
      $registry[0].reference == $reference and .platform_digest == $registry[0].platform_digest and
      .config_digest == $registry[0].config_digest and .os == "linux" and .architecture == "amd64"
    ' "$evidence/images/$role-resolution/result.json" > /dev/null
    jq -e --arg sha "$GITHUB_SHA" --arg source "https://github.com/$GITHUB_REPOSITORY" \
      --slurpfile config "$evidence/images/$role-resolution/config.json" '
      length == 1 and (.[0] | .Architecture == "amd64" and .Os == "linux" and
        .Config.Labels == $config[0].config.Labels and
        .Config.Labels["org.opencontainers.image.revision"] == $sha and
        .Config.Labels["org.opencontainers.image.source"] == $source)
    ' "$evidence/images/$role-resolution/platform-inspect.json" > /dev/null
  done
  load_ids
  bounded 30s docker tag "${image_ids[native]}" indra-native:dev
  bounded 30s docker tag "${image_ids[gateway]}" indra-gateway:dev
  bounded 30s docker tag "${image_ids[bridge]}" indra-bridge:dev
  bounded 30s docker tag "${image_ids[tika]}" tika-indra:dev
  bounded 30s docker tag "${image_ids[tesseract]}" tika-tesseract:dev
  bounded 30s docker tag "${image_ids[backend]}" indra-backend:dev
  for role in native gateway bridge; do
    case "$role" in native) binary=indra-server;; gateway) binary=indra-gateway;; bridge) binary=indra-tesseract;; esac
    mkdir "$evidence/materials/$role"
    create_material_container "${image_ids[$role]}" "$evidence/images/$role-resolution" "$evidence/images/$role-material"
    bounded 600s docker cp "$container:/relink/." "$evidence/materials/$role/"
    bounded 60s docker cp "$container:/$binary" "$evidence/bin/$binary"
    remove_material_container
    (cd "$evidence/materials/$role"; bounded 300s sha256sum --check --status SHA256SUMS)
    # Signing merges this into the SBOM of every image that ships the
    # executable; fail before the long gates.
    [[ -s "$evidence/materials/$role/static-components.spdx.json" ]] || fail "$role image lacks its static component SBOM"
    static_binary "$evidence/bin/$binary"
  done
  [[ $(wc -l < "$evidence/materials/native/licenses/native/inventory.tsv") = 32 ]] || fail 'native notice inventory is incomplete'
  [[ -s "$evidence/materials/native/licenses/native/ipp/EULA.txt" ]]
  for role in native gateway; do
    case "$role" in native) acceptance=indra-server-acceptance:test; binary=indra-server;; gateway) acceptance=indra-gateway-acceptance:test; binary=indra-gateway;; esac
    mkdir "$evidence/acceptance-build/$role"
    bounded 950s bash "$script_directory/verify-daemon-image.sh" resolve "$acceptance" "$evidence/acceptance-build/$role/resolution"
    load_acceptance_id "$role"
    create_material_container "${acceptance_ids[$role]}" "$evidence/acceptance-build/$role/resolution" "$evidence/acceptance-build/$role/material"
    bounded 300s docker cp "$container:/build-evidence/." "$evidence/acceptance-build/$role/"
    bounded 60s docker cp "$container:/$binary" "$evidence/acceptance-build/$role/$binary"
    if [[ "$role" = gateway ]]; then
      bounded 60s docker cp "$container:/indra-stack-acceptance" "$evidence/bin/indra-stack-acceptance"
      static_binary "$evidence/bin/indra-stack-acceptance"
    fi
    remove_material_container
    cmp "$evidence/bin/$binary" "$evidence/acceptance-build/$role/$binary"
  done
  # Tika and Tesseract carry the same complete bundles. Compare their embedded
  # manifests with the unique exported copies rather than archiving them thrice.
  for role in tika tesseract; do
    mkdir "$evidence/materials/$role"
    create_material_container "${image_ids[$role]}" "$evidence/images/$role-resolution" "$evidence/images/$role-material"
    for component in bridge gateway; do
      bounded 30s docker cp "$container:/relink/$component/SHA256SUMS" "$evidence/materials/$role/$component.SHA256SUMS"
      cmp "$evidence/materials/$component/SHA256SUMS" "$evidence/materials/$role/$component.SHA256SUMS"
    done
    bounded 30s docker cp "$container:/indra-gateway" "$evidence/materials/$role/indra-gateway"
    cmp "$evidence/bin/indra-gateway" "$evidence/materials/$role/indra-gateway"
    if [[ "$role" = tika ]]; then
      mkdir "$evidence/materials/tika/java"
      bounded 300s docker cp "$container:/opt/tika/." "$evidence/materials/tika/java/"
      (cd "$evidence/materials/tika/java"; bounded 120s sha256sum --check --status SHA256SUMS)
      bounded 30s docker cp "$container:/opt/indra/tesseract" "$evidence/materials/$role/indra-tesseract"
      cmp "$evidence/bin/indra-tesseract" "$evidence/materials/$role/indra-tesseract"
    fi
    remove_material_container
  done
  bounded 30s docker version > "$evidence/docker-version.txt"
  bounded 30s docker info > "$evidence/docker-info.txt"
  bounded 30s docker buildx version > "$evidence/buildx-version.txt"
  bounded 30s docker compose version > "$evidence/compose-version.txt"
  lscpu > "$evidence/lscpu.txt"
  jq -n '{schema_version:1,passed:true,scope:"Exact candidate identities, attached provenance, static Rust ELF checks, acceptance-binary equality, and source/relink inventory; not a modified-glibc relink execution"}' > "$evidence/prepare-result.json"
}

process_gates() {
  jq -e '.passed' "$evidence/prepare-result.json" > /dev/null
  [[ ! -e "$evidence/process" && ! -e "$evidence/models" ]] || fail 'refusing to overwrite process evidence'
  mkdir "$evidence/process" "$evidence/models"
  chmod 0777 "$evidence/models"
  load_ids
  load_acceptance_id native
  load_acceptance_id gateway
  for network in bridge none; do
    container=$(bounded 40s docker create --network="$network" --read-only \
      --cpus=1 --cpuset-cpus=0 --memory=512m --memory-swap=512m \
      --pids-limit=64 --cap-drop=ALL --security-opt=no-new-privileges \
      --tmpfs /tmp:rw,noexec,nosuid,nodev,size=16m,mode=1777 \
      --mount "type=bind,src=$evidence/models,dst=/models" "${image_ids[native]}" --math true models fetch)
    [[ "$container" =~ ^[a-f0-9]{64}$ ]] || fail 'model-fetch container identity is missing'
    status=0
    bounded 600s docker start --attach "$container" \
      > "$evidence/process/models-$network.json" 2> "$evidence/process/models-$network.stderr" || status=$?
    bounded 30s docker inspect "$container" > "$evidence/process/models-$network-container.json"
    bind_container "$evidence/images/native-resolution" "$evidence/process/models-$network-container.json" "$evidence/process/models-$network-binding"
    (( status == 0 )) || fail 'model-fetch command failed or exceeded its deadline'
    jq -e 'length == 1 and (.[0] |
      .RestartCount == 0 and .State.Running == false and .State.OOMKilled == false and .State.ExitCode == 0)' \
      "$evidence/process/models-$network-container.json" > /dev/null
    remove_material_container
  done
  failed=0
  for cpus in 1 2; do
    affinity=0
    [[ "$cpus" = 1 ]] || affinity=0,1
    for gate in server-unit server-native gateway-unit gateway-process; do
      if [[ "$gate" = server-* ]]; then acceptance_role=native; else acceptance_role=gateway; fi
      acceptance=${acceptance_ids[$acceptance_role]}
      # A create/start pair retains an exact ID even when a test fails or times out.
      container=$(bounded 40s docker create --network=none --read-only \
        --cpus="$cpus" --cpuset-cpus="$affinity" --memory=3g --memory-swap=3g \
        --pids-limit=128 --cap-drop=ALL --security-opt=no-new-privileges \
        --tmpfs /tmp:rw,nosuid,nodev,size=32m,mode=1777 \
        --mount "type=bind,src=$evidence/models,dst=/models,readonly" \
        --env INDRA_TEST_MODEL_DIR=/models/bd02167fdf2005a19962cc15188f8496a2c87d3b \
        --env INDRA_TEST_SERVER_BIN=/indra-server --env INDRA_TEST_THREADS="$cpus" \
        --env INDRA_TEST_GATEWAY_BIN=/indra-gateway --entrypoint "/gates/$gate" \
        "$acceptance" --include-ignored --test-threads=1 --nocapture)
      [[ "$container" =~ ^[a-f0-9]{64}$ ]] || fail 'test container identity is missing'
      status=0
      bounded 1800s docker start --attach "$container" > "$evidence/process/$gate-$cpus.log" 2>&1 || status=$?
      bounded 30s docker inspect "$container" > "$evidence/process/$gate-$cpus-container.json"
      if ! bind_container "$evidence/acceptance-build/$acceptance_role/resolution" "$evidence/process/$gate-$cpus-container.json" \
        "$evidence/process/$gate-$cpus-binding"; then failed=1; fi
      if (( status != 0 )) || ! jq -e 'length == 1 and (.[0] | .RestartCount == 0 and
        .State.Running == false and .State.OOMKilled == false and .State.ExitCode == 0)' "$evidence/process/$gate-$cpus-container.json" > /dev/null; then failed=1; fi
      bounded 40s docker rm --force "$container" > /dev/null
      container=
    done
  done
  (( failed == 0 )) || fail 'one or more real process gates failed'
  jq -n '{schema_version:1,passed:true,cpu_caps:[1,2],scope:"Real scratch native/gateway unit and process gates; production binaries byte-identical to test binaries; not a performance benchmark"}' > "$evidence/process/result.json"
}

reference_smoke() {
  jq -e '.passed' "$evidence/prepare-result.json" > /dev/null
  [[ ! -e "$evidence/references" ]] || fail 'refusing to overwrite reference evidence'
  mkdir "$evidence/references"
  load_ids
  fixture=$(realpath backend/tests/data/two-pages.tiff)
  printf '%s  %s\n' 303ebb45259b96fecc0b200edf8e0a68ed50e2b43f441bceef8694e43699c329 "$fixture" | sha256sum --check --status
  for arm in tesseract backend; do
    for cpus in 1 2; do
      run="$evidence/references/$arm-$cpus"
      mkdir "$run"
      project="indrareference${GITHUB_RUN_ID}${GITHUB_RUN_ATTEMPT}${arm}${cpus}${RANDOM}"
      [[ "$project" =~ ^[a-z0-9]{1,63}$ ]] || fail 'invalid owned project name'
      affinity=0
      [[ "$cpus" = 1 ]] || affinity=0,1
      # Both reference arms use the exact same production gateway/Tika image.
      # Gateway OCR-readiness coupling stays disabled for the Python protocol.
      jq -n --arg project "$project" --arg arm "$arm" --arg affinity "$affinity" --argjson cpus "$cpus" \
        --arg tika "${images[tika]}" --arg tesseract "${images[tesseract]}" --arg backend "${images[backend]}" '
        def common: {platform:"linux/amd64",cpuset:$affinity,pids_limit:128,read_only:true,user:"65532:65532",
          cap_drop:["ALL"],security_opt:["no-new-privileges=true"],restart:"no",tmpfs:["/tmp:rw,noexec,nosuid,nodev,size=256m,mode=1777"]};
        def frontend: common + {image:$tika,command:["serve"],cpus:($cpus/4),mem_limit:"1g",memswap_limit:"1g",stop_grace_period:"350s",
          environment:{INDRA_ADDR:"127.0.0.1:8081",INDRA_TIMEOUT_SECONDS:"70",INDRA_CONNECT_TIMEOUT_MS:"1000"}};
        {name:$project,services:(if $arm == "tesseract" then
          {tika:(frontend + {image:$tesseract,cpus:$cpus,mem_limit:"3g",memswap_limit:"3g",
            ports:["127.0.0.1:19998:9998"],environment:{OMP_THREAD_LIMIT:"1",OMP_NUM_THREADS:"1",TESSDATA_PREFIX:"/opt/tessdata"}})}
        else
          {indra:(common + {image:$backend,cpus:($cpus*3/4),mem_limit:"2g",memswap_limit:"2g",stop_grace_period:"100s",
            ports:["127.0.0.1:19998:9998"],environment:{INDRA_WORKERS:"1",INDRA_THREADS:($cpus|tostring),INDRA_REQUEST_TIMEOUT_SECONDS:"65",
              OMP_NUM_THREADS:($cpus|tostring),OMP_THREAD_LIMIT:($cpus|tostring),MKL_NUM_THREADS:"1",OPENBLAS_NUM_THREADS:"1"}}),
           tika:(frontend + {network_mode:"service:indra",depends_on:{indra:{condition:"service_started"}}})}
        end)}
      ' > "$run/compose.json"
      compose=(docker compose --project-name "$project" --file "$run/compose.json")
      for kind in container network volume; do
        case "$kind" in container) query=(docker ps --all --quiet);; network) query=(docker network ls --quiet);; volume) query=(docker volume ls --quiet);; esac
        bounded 30s "${query[@]}" --filter "label=com.docker.compose.project=$project" > "$run/preexisting-$kind.txt"
        [[ ! -s "$run/preexisting-$kind.txt" ]] || fail 'refusing to reuse an existing reference project'
      done
      compose_active=1
      bounded 60s "${compose[@]}" config --format json > "$run/rendered.json"
      jq -e --argjson cpus "$cpus" '([.services[].cpus] | add) == $cpus' "$run/rendered.json" > /dev/null
      bounded 90s "${compose[@]}" up --detach --no-build --pull never > "$run/startup.log" 2>&1
      due=$((SECONDS + 360))
      while true; do
        ready=1
        bounded 8s "${compose[@]}" exec --no-TTY tika /indra-gateway probe ready > "$run/gateway-ready.log" 2>&1 || ready=0
        if [[ "$arm" = backend ]]; then
          bounded 8s "${compose[@]}" exec --no-TTY tika /opt/indra/tesseract --ready > "$run/backend-ready.log" 2>&1 || ready=0
        fi
        (( ready == 0 )) || break
        (( SECONDS < due )) || fail 'reference startup deadline exceeded'
        sleep 1
      done
      bounded 30s "${compose[@]}" ps --all --quiet > "$run/container-ids.txt"
      mapfile -t ids < "$run/container-ids.txt"
      expected_count=1
      [[ "$arm" = tesseract ]] || expected_count=2
      [[ ${#ids[@]} = "$expected_count" ]] || fail 'unexpected reference container count'
      bounded 30s docker inspect "${ids[@]}" > "$run/before.json"
      jq -e --arg project "$project" --arg arm "$arm" '
        ([.[].Config.Labels["com.docker.compose.service"]] | sort) ==
          (if $arm == "tesseract" then ["tika"] else ["indra","tika"] end) and
        all(.[]; .Config.Labels["com.docker.compose.project"] == $project and .RestartCount == 0 and
          .State.Running and .State.OOMKilled == false)
      ' "$run/before.json" > /dev/null
      services=(tika)
      [[ "$arm" = tesseract ]] || services+=(indra)
      for service in "${services[@]}"; do
        image_role=tika
        if [[ "$arm" = tesseract ]]; then image_role=tesseract; elif [[ "$service" = indra ]]; then image_role=backend; fi
        jq -e --arg service "$service" 'map(select(.Config.Labels["com.docker.compose.service"] == $service)) |
          select(length == 1)' "$run/before.json" > "$run/$service-container.json"
        bind_container "$evidence/images/$image_role-resolution" "$run/$service-container.json" "$run/$service-binding"
      done
      bounded 20s curl --fail --silent --show-error --connect-timeout 3 --max-time 15 \
        http://127.0.0.1:19998/version > "$run/tika-version.txt"
      grep -Fq 'Apache Tika 3.3.2' "$run/tika-version.txt"
      # One real OCR request, no recovery attempts or replacement predictions.
      bounded 310s curl --fail --silent --show-error --connect-timeout 3 --max-time 300 \
        --request PUT --header 'Content-Type: image/tiff' --header 'Accept: text/plain' \
        --data-binary "@$fixture" --dump-header "$run/headers.txt" --output "$run/response.txt" \
        http://127.0.0.1:19998/tika > "$run/curl.stdout" 2> "$run/curl.stderr"
      grep -Fxq 'INVOICE 12345' "$run/response.txt"
      grep -Fxq 'SECOND PAGE 67890' "$run/response.txt"
      bounded 30s "${compose[@]}" logs --no-color > "$run/containers.log" 2>&1
      bounded 480s "${compose[@]}" stop > "$run/stop.log" 2>&1
      bounded 30s docker inspect "${ids[@]}" > "$run/after.json"
      jq -e --slurpfile before "$run/before.json" '
        ([.[].Id] | sort) == ([$before[0][].Id] | sort) and
        all(.[]; .RestartCount == 0 and .State.Running == false and .State.OOMKilled == false and .State.ExitCode == 0)
      ' "$run/after.json" > /dev/null
      bounded 600s "${compose[@]}" down --remove-orphans --volumes > "$run/removal.log" 2>&1
      compose_active=0
      jq -n --arg arm "$arm" --argjson cpus "$cpus" '{passed:true,arm:$arm,total_cpu_limit:$cpus,scope:"Two-page Tika smoke and clean stop; no latency/accuracy comparison"}' > "$run/result.json"
    done
  done
  jq -n '{schema_version:1,passed:true,arms:["tesseract","backend"],cpu_caps:[1,2]}' > "$evidence/references/result.json"
}

make_index() {
  [[ ! -e "$evidence/candidates.json" && ! -e "$evidence/evidence-files.sha256" ]] || fail 'refusing to overwrite the release index'
  for result in prepare-result.json process/result.json references/result.json; do jq -e '.passed' "$evidence/$result" > /dev/null; done
  [[ $(cat "$evidence/compose/exit-status.txt") = 0 && -s "$evidence/compose/result.txt" ]]
  [[ $(cat "$evidence/kubernetes/exit-status.txt") = 0 && -s "$evidence/kubernetes/result.txt" ]]
  bash "$script_directory/verify-release-traces.sh" "$evidence" > "$evidence/tracing/release-binding.json"
  jq -n --arg repo "$GITHUB_REPOSITORY" --arg sha "$GITHUB_SHA" --arg ref "$GITHUB_REF" \
    --arg run "$GITHUB_RUN_ID" --arg attempt "$GITHUB_RUN_ATTEMPT" --arg workflow "$GITHUB_WORKFLOW_REF" \
    --slurpfile native "$evidence/build/native/image.json" --slurpfile gateway "$evidence/build/gateway/image.json" \
    --slurpfile bridge "$evidence/build/bridge/image.json" --slurpfile tika "$evidence/build/tika/image.json" \
    --slurpfile tesseract "$evidence/build/tesseract/image.json" --slurpfile backend "$evidence/build/backend/image.json" '
    {schema_version:1,repository:$repo,source_sha:$sha,source_ref:$ref,release_run_id:$run,release_run_attempt:$attempt,
     workflow_ref:$workflow,workflow_identity:("https://github.com/"+$workflow),oidc_issuer:"https://token.actions.githubusercontent.com",
     images:[$native[0],$gateway[0],$bridge[0],$tika[0],$tesseract[0],$backend[0]]}
  ' > "$evidence/candidates.json"
  # Exclude the reusable model cache and transient named-pipe/control artifacts.
  # Release materials and every completed gate log are included before signing.
  # Its own output is explicitly excluded; nested component manifests are included.
  # shellcheck disable=SC2094
  (cd "$evidence"; find . -path ./models -prune -o -type f ! -path ./evidence-files.sha256 -print0 | sort -z | xargs -0 -r sha256sum > evidence-files.sha256)
  manifest_sha=$(sha256sum "$evidence/evidence-files.sha256" | cut -d' ' -f1)
  manifest_bytes=$(stat --format=%s "$evidence/evidence-files.sha256")
  jq -n --arg sha "$GITHUB_SHA" --arg run "$GITHUB_RUN_ID" --arg attempt "$GITHUB_RUN_ATTEMPT" \
    --arg digest "$manifest_sha" --argjson bytes "$manifest_bytes" \
    --arg artifact "signed-release-evidence-$GITHUB_SHA-$GITHUB_RUN_ID-$GITHUB_RUN_ATTEMPT" '
    {schema_version:1,source_sha:$sha,release_run_id:$run,release_run_attempt:$attempt,
     evidence_artifact:$artifact,evidence_manifest:{path:"evidence-files.sha256",sha256:$digest,bytes:$bytes},
     verification:{passed:true,checks:["raw OCI descriptor chain, provenance subject, max-mode graph and exact Dockerfile/target/source hints",
       "clean checked-out source commit and tracked-file hashes","exact candidate image IDs and source labels","static Rust ELF and source/relink inventory",
       "production and acceptance binary equality","native and gateway real process gates at 1/2 CPU",
       "two-page native Tika Compose and Kubernetes drain with original container terminal evidence",
       "exact gateway-Java-bridge-native OTLP parent graph, context isolation, privacy and shutdown flush at 1/2 CPU",
       "Tesseract and Python reference two-page Tika smoke at 1/2 CPU"]},
     scope:{purpose:"release correctness and redistribution evidence",performance_claim:false,
       sbom_limit:"The SBOMs of the native, gateway, bridge, Tika and Tesseract images add the build-time static components of each static executable they ship (glibc and libgcc; native also libstdc++, ONNX Runtime, OpenCV, IPP ICV and the OpenCV codecs) to Syft discovery; other static archive members appear only in the native build/archive and notice inventories retained in the materials.",
       source_limit:"Source/relink inputs and exact original ELF checks; no claim of executing a modified-glibc rebuild."}}
  ' > "$evidence/release-predicate.json"
}

case "$mode" in
  prepare) prepare;;
  process) process_gates;;
  references) reference_smoke;;
  index) make_index;;
  *) fail 'unknown release verification mode';;
esac
