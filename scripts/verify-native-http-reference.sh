#!/usr/bin/env bash
# Correctness only: exact production HTTP server against frozen upstream text.
set -euo pipefail
[[ $# = 8 ]] || { echo 'usage: verify-native-http-reference.sh STATIC_CLIENT TELEMETRY_IDENTITY IMAGE MODELS INPUTS RAW_REFERENCE CLIENT_BUILD_RECEIPT NEW_OUTPUT' >&2; exit 2; }
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
source "$root/scripts/lib/cgroup-admission.sh"
client=$(realpath -e "$1"); identity=$(realpath -e "$2"); image=$3; models=$(realpath -e "$4"); inputs=$(realpath -e "$5"); reference=$(realpath -e "$6"); client_receipt=$(realpath -e "$7"); output=$(realpath -m "$8")
[[ -x $client && ! -e $output && ! -L $output ]]
mkdir -m0700 "$output"
sha() { sha256sum -- "$1" | cut -d' ' -f1; }
docker_call() { timeout --signal=TERM --kill-after=5s 30s docker "$@"; }
[[ $image = sha256:cbc65a508d432318d98a53587f555bb60f54737327918ec49b3501f5c6feda92 ]]
server_sha=c9cfe911a4d1524c69dbf4f69fe0bca70bb754b9f0c8bc9087fd2c06ff8308d6
jq -e --arg image "$image" --arg server "$server_sha" '.assembled_image_root==$image and .role_sha256.native==$server' "$identity" > /dev/null
cp "$identity" "$output/telemetry-identity.json"
cp "$reference/selection-plan.json" "$output/selection-plan.json"
cp "$reference/comparison-protocol.json" "$output/comparison-protocol.json"
[[ $(sha "$output/selection-plan.json") = 6487893cfd5ad2c7ed43fb1574b6e85a85bce82131fd6df0e6df68c573eb9129 ]]
[[ $(sha "$output/comparison-protocol.json") = 049e823f888db67f475c58f1a6833c7c546e75c3184b6ddb83925dcac1e1d561 ]]
jq -e '.cases|length==14 and ([.[]|select(.math==true)|.id]==["math-on"])' "$output/selection-plan.json" > /dev/null
jq -e --arg selection "$(sha "$output/selection-plan.json")" '.selection_plan_sha256==$selection and (.reference_assets|length)==14 and .rules.text=="exact UTF-8 bytes including whitespace and page separators"' "$output/comparison-protocol.json" > /dev/null
jq -r '.cases[]|[.id,(.path|split("/")|last),.input_sha256,(.input_bytes|tostring),(.math|tostring)]|@tsv' "$output/selection-plan.json" > "$output/cases.tsv"
while IFS=$'\t' read -r id file expected bytes math; do
 [[ $id =~ ^[A-Za-z0-9_-]+$ && $file =~ ^[A-Za-z0-9_.-]+$ && -f $inputs/$file && ! -L $inputs/$file && $(sha "$inputs/$file") = "$expected" && $(stat -c%s "$inputs/$file") = "$bytes" ]]
 expected=$(jq -er --arg id "$id" '.reference_assets[$id].text' "$output/comparison-protocol.json")
 [[ -f $reference/reference/$id/text.txt && ! -L $reference/reference/$id/text.txt && $(sha "$reference/reference/$id/text.txt") = "$expected" ]]
done < "$output/cases.tsv"
[[ -z $(find "$models" -type l -print -quit) ]]
(cd "$models" && find . -type f -print0 | sort -z | xargs -0 -r sha256sum) > "$output/models.sha256"
git -C "$root" rev-parse HEAD > "$output/controller-source-sha.txt"
producer=$(cat "$client_receipt/source-sha.txt");[[ $producer =~ ^[a-f0-9]{40}$ ]]
[[ $(cut -d' ' -f1 "$client_receipt/binary.sha256") = "$(sha "$client")" ]]
printf '%s\n' "$producer" > "$output/client-producing-source-sha.txt"
cp "$client_receipt/binary.sha256" "$output/client-producing-binary.sha256"
cp "$client_receipt/build.stdout" "$output/client-build.stdout"
cp "$client_receipt/build.stderr" "$output/client-build.stderr"
git -C "$root" ls-tree -r --name-only "$producer" -- Cargo.toml Cargo.lock rust-toolchain.toml .cargo src examples/http_reference.rs shared/trace-context > "$output/client-source-equality.paths"
: > "$output/client-source-equality.sha256"
while IFS= read -r source_file; do
 expected=$(git -C "$root" show "$producer:$source_file" | sha256sum | cut -d' ' -f1)
 [[ -f $root/$source_file && $(sha "$root/$source_file") = "$expected" ]]
 printf '%s  %s\n' "$expected" "$source_file" >> "$output/client-source-equality.sha256"
done < "$output/client-source-equality.paths"
git -C "$root" diff --quiet HEAD -- Cargo.toml Cargo.lock src examples/http_reference.rs scripts/verify-native-http-reference.sh scripts/lib/cgroup-admission.sh
sha256sum "$client" "$root/examples/http_reference.rs" "$root/src/lib.rs" "$root/Cargo.lock" "$root/scripts/verify-native-http-reference.sh" > "$output/client-source-identities.sha256"
readelf -W -l "$client" > "$output/client.program-headers.txt"
readelf -W -d "$client" > "$output/client.dynamic.txt"
if grep -q INTERP "$output/client.program-headers.txt" || grep -q NEEDED "$output/client.dynamic.txt"; then exit 1; fi
jq -n --arg source "$(git -C "$root" rev-parse HEAD)" --arg producer "$producer" --arg client "$(sha "$client")" --arg image "$image" --arg server "$server_sha" --arg models "$(sha "$output/models.sha256")" '{scope:"prospective exact HTTP UTF-8 reference gate; not timing or850tensor replay",controller_source_sha:$source,client_producing_source_sha:$producer,client_sha256:$client,image_root:$image,server_sha256:$server,models_inventory_sha256:$models,cpus:[1,2],affinity:["12","12,13"],memory_bytes:3221225472,swap_bytes:0,workers:1,queue_capacity:2,batch_size:64,spinning:true,ordinary_worker:true,threads:"same asCPU cap",math_modes:[false,true],cases_per_cpu:14,client_request_deadline_seconds:420,server_request_deadline_seconds:60,server_shutdown_seconds:75,text_policy:"exact raw UTF-8 bytes including form feed; no normalization",reference_selection_sha256:"6487893cfd5ad2c7ed43fb1574b6e85a85bce82131fd6df0e6df68c573eb9129",reference_protocol_sha256:"049e823f888db67f475c58f1a6833c7c546e75c3184b6ddb83925dcac1e1d561"}' > "$output/http-reference-protocol.json"
docker_call image inspect "$image" > "$output/image.json"
jq -e --arg image "$image" 'length==1 and .[0].Id==$image and .[0].Os=="linux" and .[0].Architecture=="amd64"' "$output/image.json" > /dev/null
timeout --signal=TERM --kill-after=5s 950s bash "$root/scripts/verify-daemon-image.sh" resolve "$image" "$output/image-proof"
owner=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n'); active='' name='' failed=0
cleanup() {
 code=$?; trap - EXIT; cleanup_code=0
 if [[ -n $name ]]; then
  if docker_call inspect "$name" > "$output/interrupted.json"; then
   if jq -e --arg owner "$owner" --arg image "$image" --arg id "$active" 'length==1 and (.[0]|.Image==$image and .Config.Labels["org.tika-indra.http-reference"]==$owner and ($id=="" or .Id==$id))' "$output/interrupted.json" > /dev/null; then
    active=$(jq -er '.[0].Id' "$output/interrupted.json")
    timeout --signal=TERM --kill-after=5s 40s docker stop --timeout 30 "$active" > "$output/interrupted-stop.txt" 2>&1 || cleanup_code=1
    docker_call rm "$active" > "$output/interrupted-remove.txt" 2>&1 || cleanup_code=1
   else cleanup_code=1; fi
  else
   docker_call ps --all --quiet --filter "name=^/$name$" > "$output/interrupted-existing.txt" || cleanup_code=1
   [[ ! -s $output/interrupted-existing.txt ]] || cleanup_code=1
  fi
 fi
 [[ $cleanup_code = 0 ]] || code=1
 jq -n --argjson exit "$code" --argjson cleanup "$cleanup_code" '{scope:"exact HTTP UTF-8 reference correctness only; not850tensors or timing",exit:$exit,cleanup_exit:$cleanup}' > "$output/status.json"
 (cd "$output" && find . -type f ! -path ./SHA256SUMS -print0 | sort -z | xargs -0 -r sha256sum > SHA256SUMS)
 exit "$code"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
snapshot() {
 local destination=$1
 mkdir "$destination"
 for key in cpu.max cpu.stat cpuset.cpus.effective memory.max memory.swap.max memory.current memory.peak memory.events pids.max pids.current cgroup.procs; do cp "$group/$key" "$destination/$key"; done
 cp "/proc/$pid/status" "$destination/server.status"
}
for cpus in 1 2; do
 affinity=12; [[ $cpus = 1 ]] || affinity=12,13
 for math in false true; do
  cell="$output/cpu-$cpus-math-$math";mkdir "$cell"
  name="indra-http-reference-$owner-$cpus-$math"
  server_args=(--listen 127.0.0.1:8081 --model-dir /models --workers 1 --queue-capacity 2 --threads "$cpus" --cv-threads "$cpus" --batch-size 64 --math "$math" --intra-op-spinning true --inter-op-spinning true --experimental-continuous-batching false --request-timeout-seconds 60 --shutdown-timeout-seconds 75)
  docker_call create --name "$name" --label "org.tika-indra.http-reference=$owner" --pull=never --entrypoint /http-reference \
   --network=none --read-only --cgroupns=private --user "$(id -u):$(id -g)" --cpus "$cpus" --cpuset-cpus "$affinity" \
   --memory=3g --memory-swap=3g --pids-limit=256 --cap-drop=ALL --security-opt=no-new-privileges --ulimit stack=8388608:8388608 \
   --tmpfs /tmp:rw,nosuid,nodev,size=64m,mode=1777 \
   --mount "type=bind,src=$client,dst=/http-reference,readonly" --mount "type=bind,src=$models,dst=/models,readonly" \
   --mount "type=bind,src=$inputs,dst=/inputs,readonly" --mount "type=bind,src=$cell,dst=/evidence" "$image" \
   hold /evidence/admitted /indra-server "${server_args[@]}" serve > "$cell/container.id"
  active=$(cat "$cell/container.id");[[ $active =~ ^[a-f0-9]{64}$ ]]
  docker_call inspect "$active" > "$cell/container-before.json"
  jq -e --arg image "$image" --arg owner "$owner" --arg affinity "$affinity" --argjson cpus "$cpus" 'length==1 and (.[0]|.Image==$image and .Config.Labels["org.tika-indra.http-reference"]==$owner and .HostConfig.NanoCpus==($cpus*1000000000) and .HostConfig.CpusetCpus==$affinity and .HostConfig.Memory==3221225472 and .HostConfig.MemorySwap==3221225472 and .HostConfig.CgroupnsMode=="private" and .HostConfig.PidsLimit==256 and .HostConfig.NetworkMode=="none" and .HostConfig.PidMode=="" and .HostConfig.Privileged==false and .HostConfig.CapDrop==["ALL"] and (.HostConfig.CapAdd//[])==[] and .HostConfig.SecurityOpt==["no-new-privileges"] and .HostConfig.ReadonlyRootfs==true and .Config.Entrypoint==["/http-reference"] and (.HostConfig.Ulimits|any(.Name=="stack" and .Soft==8388608 and .Hard==8388608)))' "$cell/container-before.json" > "$cell/resource-admission.txt"
  bash "$root/scripts/verify-daemon-image.sh" bind "$output/image-proof" "$cell/container-before.json" "$cell/executed-identity"
  docker_call start "$active" > "$cell/started.txt"
  docker_call inspect "$active" > "$cell/container-running.json";pid=$(jq -er '.[0].State.Pid|select(.>0)' "$cell/container-running.json")
  cp "/proc/$pid/cgroup" "$cell/host-process.cgroup";relative=$(awk -F: '$1=="0"{print $3}' "$cell/host-process.cgroup")
  [[ $relative = /* && $(basename "$relative") = "docker-$active.scope" ]]
  group=$(readlink -e "/sys/fs/cgroup$relative");attest_host_ancestors "$group" "$cpus" "$cell/host-ancestors.txt"
  [[ $(cat "$group/cpuset.cpus.effective") = "${affinity/,/-}" && $(cat "$group/cpu.max") = "$((cpus*100000)) 100000" && $(cat "$group/memory.max") = 3221225472 && $(cat "$group/memory.swap.max") = 0 ]]
  snapshot "$cell/before"
  [[ $(awk '$1=="Cpus_allowed_list:"{print $2}' "$cell/before/server.status") = "${affinity/,/-}" ]]
  cp "/proc/$pid/limits" "$cell/server.limits"
  awk '$1=="Max" && $2=="stack" && $3=="size" {found++;if($4!=8388608 || $5!=8388608)exit 1} END{if(found!=1)exit 1}' "$cell/server.limits"
  docker_call exec "$active" /http-reference copy /indra-server | sha256sum > "$cell/executed-server.sha256"
  [[ $(cut -d' ' -f1 "$cell/executed-server.sha256") = "$server_sha" ]]
  docker_call exec "$active" /http-reference copy /http-reference | sha256sum > "$cell/executed-client.sha256"
  [[ $(cut -d' ' -f1 "$cell/executed-client.sha256") = "$(sha "$client")" ]]
  docker_call exec "$active" /indra-server --help > "$cell/server-help.stdout" 2> "$cell/server-help.stderr"
  docker_call exec "$active" /indra-server "${server_args[@]}" check-config > "$cell/server-check-config.stdout" 2> "$cell/server-check-config.stderr"
  printf 'admitted\n' > "$cell/admitted.next";mv "$cell/admitted.next" "$cell/admitted"
  ready=0
  for ((attempt=0;attempt<120;attempt++)); do
   if docker_call exec "$active" /http-reference ready 127.0.0.1:8081 >> "$cell/readiness.stdout" 2>> "$cell/readiness.stderr"; then ready=1;break; fi
   docker_call inspect "$active" > "$cell/readiness-state.json"
   if [[ $(jq -r '.[0].State.Running' "$cell/readiness-state.json") != true ]]; then
    docker_call logs "$active" > "$cell/startup-failure.stdout" 2> "$cell/startup-failure.stderr"
    break
   fi
   sleep 1
  done
  [[ $ready = 1 ]]
  while IFS=$'\t' read -r id file expected bytes selected_math; do
   [[ $selected_math = "$math" ]] || continue
   case_dir="$cell/$id";mkdir "$case_dir";request_exit=0
   snapshot "$case_dir/before"
   timeout --signal=TERM --kill-after=5s 430s docker exec "$active" /http-reference request 127.0.0.1:8081 "/inputs/$file" "/evidence/$id/body.txt" "/evidence/$id/wire.http" > "$case_dir/stdout" 2> "$case_dir/stderr" || request_exit=$?
   expected_text=$(jq -er --arg id "$id" '.reference_assets[$id].text' "$output/comparison-protocol.json")
   matched=false
   if [[ $request_exit = 0 ]] && cmp -s "$case_dir/body.txt" "$reference/reference/$id/text.txt"; then matched=true;else failed=1;fi
   jq -n --arg id "$id" --argjson cpus "$cpus" --argjson math "$math" --argjson exit "$request_exit" --argjson matched "$matched" --arg expected "$expected_text" --arg actual "$(sha "$case_dir/body.txt")" '{id:$id,cpus:$cpus,math:$math,client_exit:$exit,exact_utf8_match:$matched,expected_sha256:$expected,actual_sha256:$actual}' > "$case_dir/result.json"
   snapshot "$case_dir/after"
  done < "$output/cases.tsv"
  snapshot "$cell/after"
  docker_call logs "$active" > "$cell/server.stdout" 2> "$cell/server.stderr"
  timeout --signal=TERM --kill-after=5s 40s docker stop --timeout 30 "$active" > "$cell/stopped.txt"
  docker_call inspect "$active" > "$cell/container-after.json"
  jq -e 'length==1 and (.[0].State|.Running==false and .OOMKilled==false and .ExitCode==0)' "$cell/container-after.json" > "$cell/terminal-admission.txt"
  docker_call logs "$active" > "$cell/server-final.stdout" 2> "$cell/server-final.stderr"
  docker_call rm "$active" > "$cell/removed.txt";active='';name=''
 done
done
# Only the prospectively frozen case paths participate. OCI identity helpers
# also emit result.json, so a broad recursive result glob is invalid here.
result_files=()
for cpus in 1 2; do
 while IFS=$'\t' read -r id file expected bytes math; do
  result_files+=("$output/cpu-$cpus-math-$math/$id/result.json")
 done < "$output/cases.tsv"
done
jq -s --arg server "$server_sha" --arg image "$image" '{scope:"28 exact actual HTTP-server UTF-8 responses vs existing frozen upstream text; no850tensor or timing claim",server_sha256:$server,image_root:$image,cases:.,passed:(length==28 and all(.[];.client_exit==0 and .exact_utf8_match==true))}' "${result_files[@]}" > "$output/summary.json"
jq -e '.passed==true' "$output/summary.json" > /dev/null
[[ $failed = 0 ]]
