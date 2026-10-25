#!/usr/bin/env bash
# Control fixtures only: no timings, OCR outputs or performance claims are generated.
set -euo pipefail
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
temporary=$(mktemp -d /tmp/indra-ablation-control.XXXXXXXX)
trap 'rm -rf -- "$temporary"' EXIT
bash "$root/scripts/ablate-native-docker.sh" --schedule > "$temporary/schedule.json"
jq -e 'length==128 and ([.[].id]|unique|length)==128 and
  ([.[].cpus]|unique)==[1,2] and ([.[].fixture]|unique)==["math","tiff"] and
  (group_by([.cpus,.fixture,.comparison,.block])|length==64 and all(.[];
    length==2 and (map(.arm)|sort)==["A","B"] and .[0].order==.[1].order)) and
  (group_by([.cpus,.fixture,.comparison])|all(.[];
    (map(select(.arm=="A"))|map(.order))==["AB","BA","BA","AB"])) and
  all(.[]; if .arm=="A" then .settings=={lto:"thin",spinning:true,batch_size:64} else
    if .comparison=="fat-lto" then .settings=={lto:"fat",spinning:true,batch_size:64}
    elif .comparison=="spinning-off" then .settings=={lto:"thin",spinning:false,batch_size:64}
    elif .comparison=="batch-1" then .settings=={lto:"thin",spinning:true,batch_size:1}
    else .settings=={lto:"thin",spinning:true,batch_size:8} end end)' "$temporary/schedule.json" > /dev/null
# The 2026-10-18 schedule is unchanged by the generalized generator.
jq -e --slurpfile sealed "$root/runtime/validation/native-ablations/2026-10-18/schedule.json" '. == $sealed[0]' \
    "$temporary/schedule.json" > /dev/null
ablation_protocol=$root/runtime/validation/native-ablation-protocol.json
thread_protocol=$root/runtime/validation/native-thread-study-protocol.json
for protocol in "$ablation_protocol" "$thread_protocol"; do
    if printf '[]\n' | jq --slurpfile protocol "$protocol" -f "$root/scripts/summarize-native-ablations.jq" \
        > "$temporary/empty.json" 2> "$temporary/empty.stderr"; then exit 1; fi
done

# Thread-policy schedule: three placements, two fixtures, four comparisons against A.
bash "$root/scripts/ablate-native-docker.sh" --threads --schedule > "$temporary/threads.json"
jq -e 'length==192 and ([.[].id]|unique|length)==192 and
  ([.[].placement]|unique)==["distinct-cores","shipped-ocr-quota","smt-siblings"] and
  all(.[]; (.millicpus==(if .placement=="shipped-ocr-quota" then 1500 else 2000 end)) and
    .cpuset==(if .placement=="distinct-cores" then "distinct-cores" else "smt-siblings" end)) and
  ([.[].fixture]|unique)==["math","tiff"] and
  (group_by([.placement,.fixture,.comparison,.block])|length==96 and all(.[];
    length==2 and (map(.arm)|sort)==["A","B"] and .[0].order==.[1].order)) and
  (group_by([.placement,.fixture,.comparison])|all(.[];
    (map(select(.arm=="A"))|map(.order))==["AB","BA","BA","AB"])) and
  all(.[]; .settings==(if .arm=="A" then {threads:2,cv_threads:2,spinning:true} else
    {"B-t2-cv1":{threads:2,cv_threads:1,spinning:true},"C-t1-cv1":{threads:1,cv_threads:1,spinning:true},
     "D-spinning-off":{threads:2,cv_threads:2,spinning:false},"E-t1-cv2":{threads:1,cv_threads:2,spinning:true}}[.comparison]
    end + {batch_size:64,lto:"thin"}))' "$temporary/threads.json" > /dev/null

# Placements come from sysfs topology; CPU:core:package:thread_siblings_list.
topology() {
    local directory=$1 entry cpu core package siblings
    shift
    for entry in "$@"; do
        IFS=: read -r cpu core package siblings <<< "$entry"
        mkdir -p "$directory/cpu$cpu/topology"
        printf '%s\n' "$siblings" > "$directory/cpu$cpu/topology/thread_siblings_list"
        printf '%s\n' "$core" > "$directory/cpu$cpu/topology/core_id"
        printf '%s\n' "$package" > "$directory/cpu$cpu/topology/physical_package_id"
    done
}
placements() { bash "$root/scripts/ablate-native-docker.sh" --threads --placements "$@"; }
topology "$temporary/smt" 0:0:0:0-1 1:0:0:0-1 2:1:0:2-3 3:1:0:2-3
[[ $(placements "$temporary/smt" 0 1 2 3 | jq -c .) = '{"cpu-1":"0","cpu-2":"0,1","smt-siblings":"0,1","distinct-cores":"0,2"}' ]]
[[ $(placements "$temporary/smt" 1 2 3 | jq -c .) = '{"cpu-1":"2","cpu-2":"2,3","smt-siblings":"2,3","distinct-cores":"1,2"}' ]]
topology "$temporary/interleaved" 0:0:0:0,2 1:1:0:1,3 2:0:0:0,2 3:1:0:1,3
[[ $(placements "$temporary/interleaved" 0 1 2 3 | jq -c .) = '{"cpu-1":"0","cpu-2":"0,2","smt-siblings":"0,2","distinct-cores":"0,1"}' ]]
topology "$temporary/no-smt" 0:0:0:0 1:1:0:1
topology "$temporary/one-core" 0:0:0:0-1 1:0:0:0-1 2:0:1:2-3 3:0:1:2-3
topology "$temporary/inconsistent" 0:0:0:0-1 1:1:0:1 2:2:0:2
rejected_placement() {
    local message=$1
    shift
    if placements "$@" > "$temporary/placement.stdout" 2> "$temporary/placement.stderr"; then
        echo "placement without '$message' was accepted" >&2; exit 1
    fi
    grep -Fq -- "$message" "$temporary/placement.stderr"
}
rejected_placement 'no two allowed CPUs are SMT siblings of one core' "$temporary/no-smt" 0 1
rejected_placement "no allowed CPU is on another physical core of CPU 0's package" "$temporary/smt" 0 1
rejected_placement "no allowed CPU is on another physical core of CPU 0's package" "$temporary/one-core" 0 1 2 3
rejected_placement 'disagree about their shared core' "$temporary/inconsistent" 0 1 2
rejected_placement 'CPU 4 reports no sysfs topology' "$temporary/smt" 0 1 2 3 4

# Pre-registered decisions on synthetic process summaries: ratios are chosen, not measured.
# Arguments: the distinct-cores C median ratio and optional effect overrides.
synthetic() {
    local overrides='{}'
    if [[ $# = 2 ]]; then overrides=$2; fi
    jq --argjson distinct_c "$1" --argjson overrides "$overrides" '
      ({"smt-siblings":{"B-t2-cv1":[0.97,0.90],"C-t1-cv1":[0.96,0.55],"D-spinning-off":[0.93,0.80],"E-t1-cv2":[0.94,0.70]},
       "distinct-cores":{"B-t2-cv1":[1.00,0.90],"C-t1-cv1":[$distinct_c,0.55],"D-spinning-off":[1.03,0.80],"E-t1-cv2":[1.20,0.70]},
       "shipped-ocr-quota":{"B-t2-cv1":[0.97,0.90],"C-t1-cv1":[0.80,0.55],"D-spinning-off":[0.90,0.80],"E-t1-cv2":[0.99,0.80]}} * $overrides) as $effects |
      map(. as $row | (if .arm=="A" then [1,1] else $effects[.placement][.comparison] end) as [$m,$c] |
        # Detection slows with one ORT thread at both SMT placements; only the
        # orientation stage separates them.
        (if .arm=="B" and .placement!="distinct-cores" and .fixture=="math" then
           {"C-t1-cv1":[1,(if .placement=="smt-siblings" then 1.02 else 0.9 end),1.1,0.9,0.95,1,1],
            "B-t2-cv1":[$m,$m,0.98,$m,$m,$m,$m]}[.comparison] // [range(7)|$m]
         else [range(7)|$m] end) as $stages |
        . + {p50_us:(100000*$m),
             p95_us:(120000*(if .arm=="B" and .placement=="smt-siblings" and .comparison=="D-spinning-off"
               and .fixture=="math" and .block==2 then 1.06 else $m end)),
             # One faster spinning-off pair at the shipped quota needs 1.11 times the memory.
             phase_peak_memory_bytes:(if .arm=="B" and .placement=="shipped-ocr-quota" and .comparison=="D-spinning-off"
               and .fixture=="tiff" and .block==1 then 222000000 else 200000000 end),
             loop_cpu_us_per_request:(50000*$c),loop_cpu_system_us:1000,
             loop_throttled_us_per_request:0,loop_switches_per_request:100,
             stage_p50_us:([10000,20000,30000,40000,50000,1000,1000]|to_entries|map(.value*$stages[.key]))})' \
        "$temporary/threads.json"
}
decide() { synthetic "$1" | jq --slurpfile protocol "$thread_protocol" -f "$root/scripts/summarize-native-ablations.jq"; }
decide 1.40 > "$temporary/guarded.json"
jq -e '.decisions as $d | ($d.placements|map({key:.placement,value:[.selected,.class]})|from_entries) ==
    {"smt-siblings":["E-t1-cv2","faster"],"distinct-cores":["A",null],"shipped-ocr-quota":["C-t1-cv1","faster"]} and
  ([$d.placements[]|select(.placement=="smt-siblings").candidates[]|select(.comparison=="C-t1-cv1")][0]|
    .class=="cheaper" and .promoted) and
  ([$d.placements[]|select(.placement=="smt-siblings").candidates[]|select(.comparison=="D-spinning-off")][0]|
    .class=="faster" and .guards==false and .promoted==false) and
  ([$d.placements[]|select(.placement=="shipped-ocr-quota").candidates[]|select(.comparison=="E-t1-cv2")][0]|
    .class=="cheaper" and .promoted) and
  ([$d.placements[]|select(.placement=="shipped-ocr-quota").candidates[]|select(.comparison=="D-spinning-off")][0]|
    .class=="faster" and .guards==false and .promoted==false) and
  $d.shipped_two_cpu_profile.status=="keep A" and $d.shipped_two_cpu_profile.setting=="A" and
  ($d.shipped_two_cpu_profile.reason|test("C-t1-cv1 is promoted")) and
  ($d.per_session_detector|map({key:.placement,value:.worth_implementing})|from_entries) ==
    {"smt-siblings":false,"distinct-cores":false,"shipped-ocr-quota":true} and
  ($d.baseline_by_placement|length==6 and all(.[]; .processes==16 and .median_p50_us==100000)) and
  (.pairs|length==96)' "$temporary/guarded.json" > /dev/null
# Engine pairs alone only propose a shipped change, up to the distinct-cores bound.
for distinct in 1.04 1.05; do
    decide "$distinct" > "$temporary/proposed.json"
    jq -e '.decisions.shipped_two_cpu_profile|.status=="candidate for whole-Tika confirmation" and .setting=="C-t1-cv1" and
      .files==["compose.2cpu.yaml","deploy/overlays/2cpu/resources.yaml"] and (.reason|test("whole-Tika confirmation"))' \
        "$temporary/proposed.json" > /dev/null
done
jq -r -f "$root/scripts/native-thread-study-table.jq" "$temporary/proposed.json" > "$temporary/proposed.md"
grep -Fq -- '- Shipped two-CPU profile: C-t1-cv1 is a candidate for whole-Tika confirmation before any change to compose.2cpu.yaml and deploy/overlays/2cpu/resources.yaml;' \
    "$temporary/proposed.md"
jq -r -f "$root/scripts/native-thread-study-table.jq" "$temporary/guarded.json" > "$temporary/table.md"
grep -Fxq -- '- shipped-ocr-quota: selected C-t1-cv1 (faster).' "$temporary/table.md"
grep -Fq -- '- Shipped two-CPU profile: keep A; C-t1-cv1 is promoted in shipped-ocr-quota' "$temporary/table.md"
[[ $(grep -c '^| smt-siblings (2 CPU, smt-siblings) |' "$temporary/table.md") = 8 ]]
# Equally fast cheaper candidates: the lower largest CPU ratio wins before protocol order.
synthetic 1.40 '{"smt-siblings":{"B-t2-cv1":[0.98,0.80],"C-t1-cv1":[0.98,0.60],"E-t1-cv2":[0.98,0.70]}}' |
    jq --slurpfile protocol "$thread_protocol" -f "$root/scripts/summarize-native-ablations.jq" > "$temporary/tied.json"
jq -e '.decisions.placements[]|select(.placement=="smt-siblings")|.selected=="C-t1-cv1" and .class=="cheaper"' \
    "$temporary/tied.json" > /dev/null
# Each per-session detector condition alone rejects the option at shipped-ocr-quota.
synthetic 1.40 > "$temporary/synthetic.json"
detector() {
    jq --arg c "$1" --argjson s "$2" --argjson r "$3" 'map(if .placement=="shipped-ocr-quota" and .fixture=="math"
        and .arm=="B" and .comparison==$c then .stage_p50_us[$s]=([10000,20000,30000,40000,50000,1000,1000][$s]*$r) else . end)' \
        "$temporary/synthetic.json" | jq --slurpfile protocol "$thread_protocol" -f "$root/scripts/summarize-native-ablations.jq" |
        jq -r '.decisions.per_session_detector[]|select(.placement=="shipped-ocr-quota").worth_implementing'
}
for case in 'C-t1-cv1 2 1.05 true' 'C-t1-cv1 2 1.04 false' 'C-t1-cv1 1 1.01 false' 'C-t1-cv1 3 1.01 false' \
    'C-t1-cv1 4 1.01 false' 'B-t2-cv1 2 1.00 true' 'B-t2-cv1 2 1.02 false'; do
    read -r comparison stage ratio expected <<< "$case"
    [[ $(detector "$comparison" "$stage" "$ratio") = "$expected" ]] || { echo "per-session detector case '$case' failed" >&2; exit 1; }
done
synthetic 1.40 | jq '.[1:]' | if jq --slurpfile protocol "$thread_protocol" -f "$root/scripts/summarize-native-ablations.jq" \
    > /dev/null 2>&1; then echo 'an incomplete thread study was summarized' >&2; exit 1; fi
synthetic 1.40 | jq '.[1].arm="A"' | if jq --slurpfile protocol "$thread_protocol" -f "$root/scripts/summarize-native-ablations.jq" \
    > /dev/null 2>&1; then echo 'an unpaired thread study was summarized' >&2; exit 1; fi

# Admission of a complete synthetic evidence tree, then of single retained-file mutations.
study=$temporary/study
mkdir -p "$study"
cp "$thread_protocol" "$study/protocol.json"
cp "$temporary/threads.json" "$study/schedule.json"
jq -n '{exit:0,interrupted_controller_exit:0}' > "$study/status.json"
jq -n '{completed:true,processes:192,requests:12288,scope:"native synthetic fixture thread-policy study; not total Tika"}' > "$study/completion.json"
jq -n '[{cpu:0,core_id:0,package:0,siblings:"0-1\n"},{cpu:1,core_id:0,package:0,siblings:"0-1\n"},
  {cpu:2,core_id:1,package:0,siblings:"2-3\n"},{cpu:3,core_id:1,package:0,siblings:"2-3\n"}]' > "$study/topology.json"
jq -f "$root/scripts/native-thread-placements.jq" "$study/topology.json" > "$study/resource-selection.json"
cell() {
    local directory=$1 phase=$2 millicpus=$3 cpuset=$4
    mkdir -p "$phase/before" "$phase/after"
    jq -n '{exit:0,cleanup_exit:0}' > "$directory/status.json"
    jq -n '{completed:true}' > "$directory/completion.json"
    jq -n '{exit:0,resource_capture_exit:0}' > "$phase/phase-status.json"
    jq -n --argjson m "$millicpus" --arg c "$cpuset" '[{HostConfig:{NanoCpus:($m*1000000),CpusetCpus:$c}}]' > "$phase/container-before.json"
    printf '%s 100000\n' "$((millicpus * 100))" > "$phase/before/cpu.max"
    tr , '\n' <<< "$cpuset" > "$phase/cgroup-cpus.txt"
}
for cpus in 1 2; do
    cpuset=$(jq -er --arg role "cpu-$cpus" '.[$role]' "$study/resource-selection.json")
    phase=$study/gates-thin-$cpus/cell/gates
    cell "$study/gates-thin-$cpus" "$phase" "$((cpus * 1000))" "$cpuset"
    for gate in unit native allocations perf-control ocr-cli core-allocations telemetry-allocations model-allocations; do
        printf '0\n' > "$phase/$gate.exit"
    done
done
: > "$study/processes.jsonl"
while IFS= read -r row; do
    IFS=$'\t' read -r id millicpus role fixture threads cv spin < <(jq -r \
        '[.id,.millicpus,.cpuset,.fixture,.settings.threads,.settings.cv_threads,.settings.spinning]|@tsv' <<< "$row")
    cpuset=$(jq -er --arg role "$role" '.[$role]' "$study/resource-selection.json")
    phase=$study/$id/cell/ablation
    cell "$study/$id" "$phase" "$millicpus" "$cpuset"
    mkdir "$phase/loop-enable" "$phase/loop-disable"
    printf '0\n' > "$phase/recognition.exit"
    printf '/bundle/ocr-profiling /bundle/fixtures/input --model-dir /models --threads %s --cv-threads %s --batch-size 64 --intra-op-spinning %s --inter-op-spinning %s --warmup --iterations 64 --timeout-seconds 60 \n' \
        "$threads" "$cv" "$spin" "$spin" > "$phase/recognition.command.txt"
    expected='two-pages.expected.txt'
    if [[ $fixture = math ]]; then expected='math-page.expected.txt'; fi
    cp "$root/runtime/crates/indra-engine/tests/data/$expected" "$phase/output.txt"
    jq -nc --argjson t "$threads" --argjson c "$cv" --argjson s "$spin" 'range(64) as $i|{iteration:$i,elapsed_us:(1000+$i),
        threads:$t,cv_threads:$c,intra_op_spinning:$s,inter_op_spinning:$s,valid_columns:1,padded_columns:1,stage_us:[1,2,3,4,5,6,7]}' \
        > "$phase/requests.jsonl"
    printf 'usage_usec 10\nthrottled_usec 0\n' > "$phase/before/cpu.stat"
    printf 'usage_usec 20\nuser_usec 12\nsystem_usec 8\nnr_periods 1\nnr_throttled 0\nthrottled_usec 0\n' > "$phase/loop-enable/cpu.stat"
    printf 'usage_usec 84\nuser_usec 60\nsystem_usec 24\nnr_periods 9\nnr_throttled 2\nthrottled_usec 3\n' > "$phase/loop-disable/cpu.stat"
    printf 'usage_usec 90\nthrottled_usec 3\n' > "$phase/after/cpu.stat"
    printf '131923968\n' > "$phase/after/memory.peak"
    printf '7\tocr-profiling\t10\t2\n' > "$phase/loop-enable/threads.tsv"
    printf '7\tocr-profiling\t42\t3\n' > "$phase/loop-disable/threads.tsv"
    jq -n --argjson metadata "$row" --argjson iterations 64 --slurpfile requests "$phase/requests.jsonl" \
        --rawfile before "$phase/before/cpu.stat" --rawfile after "$phase/after/cpu.stat" --rawfile peak "$phase/after/memory.peak" \
        --rawfile loop_before "$phase/loop-enable/cpu.stat" --rawfile loop_after "$phase/loop-disable/cpu.stat" \
        --rawfile threads_before "$phase/loop-enable/threads.tsv" --rawfile threads_after "$phase/loop-disable/threads.tsv" \
        --from-file "$root/scripts/native-ablation-resources.jq" >> "$study/processes.jsonl"
done < <(jq -c '.[]' "$study/schedule.json")
jq -s --slurpfile protocol "$thread_protocol" -f "$root/scripts/summarize-native-ablations.jq" "$study/processes.jsonl" > "$study/analysis.json"
bash "$root/scripts/admit-native-thread-study.sh" "$study" > "$temporary/admission.md"
grep -Fxq 'PASS: 192 processes, 12288 exact recognitions, 2 native gate roles' "$temporary/admission.md"
grep -Fxq -- '- shipped-ocr-quota: selected A.' "$temporary/admission.md"
mutated() {
    local file=$1 message=$2
    shift 2
    cp "$study/$file" "$temporary/original"
    "$@" "$study/$file"
    if bash "$root/scripts/admit-native-thread-study.sh" "$study" > /dev/null 2> "$temporary/mutation.stderr"; then
        echo "mutated $file was admitted" >&2; exit 1
    fi
    grep -Fq -- "$message" "$temporary/mutation.stderr" || { cat "$temporary/mutation.stderr" >&2; exit 1; }
    cp "$temporary/original" "$study/$file"
}
append_byte() { printf 'x' >> "$1"; }
other_cpuset() { jq '.[0].HostConfig.CpusetCpus="0,3"' "$1" > "$1.next"; mv "$1.next" "$1"; }
other_threads() { sed -i 's/--threads 2 /--threads 1 /' "$1"; }
other_process() { jq -sc '.[4].p50_us-=1|.[]' "$1" > "$1.next"; mv "$1.next" "$1"; }
other_selection() { jq '."distinct-cores"="0,3"' "$1" > "$1.next"; mv "$1.next" "$1"; }
exited_thread() { printf '8\tocr-profiling\t1\t1\n' >> "$1"; }
other_decision() { jq '.decisions.placements[0].selected="D-spinning-off"' "$1" > "$1.next"; mv "$1.next" "$1"; }
failed_gate() { printf '1\n' > "$1"; }
other_nano_cpus() { jq '.[0].HostConfig.NanoCpus=1500000000' "$1" > "$1.next"; mv "$1.next" "$1"; }
other_quota() { printf '150000 100000\n' > "$1"; }
other_cpus() { printf '0\n2\n' > "$1"; }
other_iterations() { sed -i 's/--iterations 64 /--iterations 32 /' "$1"; }
dropped_request() { sed -i '$d' "$1"; }
other_journal() { jq -c 'if .iteration==7 then .cv_threads=3-.cv_threads else . end' "$1" > "$1.next"; mv "$1.next" "$1"; }
mutated protocol.json 'the recorded protocol is not the registered one' append_byte
mutated cell-7/cell/ablation/output.txt 'cell-7 output differs from the reference text' append_byte
mutated cell-9/cell/ablation/container-before.json 'ran with another quota or cpuset' other_cpuset
mutated cell-10/cell/ablation/container-before.json 'ran with another quota or cpuset' other_nano_cpus
mutated cell-11/cell/ablation/before/cpu.max 'has another cgroup quota' other_quota
mutated cell-13/cell/ablation/cgroup-cpus.txt 'has another effective cpuset' other_cpus
mutated cell-1/cell/ablation/recognition.command.txt 'cell-1 ran other settings' other_threads
mutated cell-2/cell/ablation/recognition.command.txt 'cell-2 ran other settings' other_iterations
mutated cell-5/cell/ablation/requests.jsonl 'cell-5 journal settings differ' dropped_request
mutated cell-6/cell/ablation/requests.jsonl 'cell-6 journal settings differ' other_journal
mutated processes.jsonl 'recorded process summaries differ from their raw files' other_process
mutated resource-selection.json 'the selected cpusets differ from the recorded topology' other_selection
mutated cell-3/cell/ablation/loop-enable/threads.tsv 'cell-3 resource counters are invalid' exited_thread
mutated analysis.json 'recorded analysis differs from the recomputed decisions' other_decision
mutated gates-thin-2/cell/gates/native.exit 'native gate native failed at 2 CPU' failed_gate
mutated gates-thin-1/cell/gates/unit.exit 'native gate unit failed at 1 CPU' failed_gate
[[ ${1:-} = --docker ]] || { [[ $# = 0 ]]; echo 'Offline ablation and thread-study schedules, placements, decisions, admission and missing-data rejection passed'; exit 0; }
[[ $# = 2 ]] || { echo 'usage: test-native-ablation-control.sh [--docker TOOLS_IMAGE]' >&2; exit 2; }
image=$2
repo=$temporary/repo; bundle=$temporary/bundle
mkdir -p "$repo/scripts" "$repo/docker" "$repo/runtime/validation" "$repo/src" "$bundle/gates" "$bundle/fixtures" "$bundle/relink" "$bundle/build-evidence" "$temporary/models"
cp "$root/scripts/profile-native.sh" "$root/scripts/profile-native-docker.sh" "$root/scripts/verify-native-profile-samples.sh" "$repo/scripts/"
mkdir -p "$repo/scripts/lib"
cp "$root/scripts/lib/profile-volume.jq" "$root/scripts/lib/profile-capabilities.jq" "$repo/scripts/lib/"
cp "$root/docker/profile-tools.Dockerfile" "$repo/docker/"
cp "$root/runtime/validation/native-profile-protocol.json" "$root/runtime/validation/native-ablation-protocol.json" "$repo/runtime/validation/"
cat > "$repo/src/main.rs" <<'RUST'
// Never emits timings or OCR. Validate the actual controlled native invocation,
// then deliberately fail; the controller must preserve failure and clean up.
use std::{fs::OpenOptions, io::{Read, Write}};

fn main() {
    let args: Vec<_> = std::env::args().collect();
    for flag in ["--model-dir", "--threads", "--cv-threads", "--batch-size",
                 "--intra-op-spinning", "--inter-op-spinning", "--warmup",
                 "--iterations", "--expect", "--expect-each", "--perf-control",
                 "--perf-ack", "--output", "--stats"] {
        assert!(args.iter().any(|arg| arg == flag), "missing {flag}");
    }
    let value = |flag| args[args.iter().position(|arg| arg == flag).unwrap()+1].as_str();
    assert!(matches!(value("--threads"), "1" | "2"));
    assert!(matches!(value("--cv-threads"), "1" | "2"));
    if value("--batch-size") == "1" {
        // Complete the counter exchange, then leave an empty journal: a
        // deliberate admission failure, never timing data.
        let mut ack = OpenOptions::new().read(true).open(value("--perf-ack")).unwrap();
        let mut control = OpenOptions::new().write(true).open(value("--perf-control")).unwrap();
        for command in [&b"enable\n"[..], b"disable\n"] {
            control.write_all(command).unwrap();
            let mut reply = [0_u8; 5];
            ack.read_exact(&mut reply).unwrap();
            assert_eq!(&reply, b"ack\n\0");
        }
        std::fs::copy(value("--expect"), value("--output")).unwrap();
        std::fs::File::create(value("--stats")).unwrap();
        return;
    }
    assert_eq!(value("--batch-size"),"8");
    assert!(matches!(value("--iterations"), "32" | "64"));
    assert_eq!(value("--intra-op-spinning"),"false");
    assert_eq!(value("--inter-op-spinning"),"false");
    eprintln!("EXPECTED CONTROL FIXTURE EXIT 43: NO OCR OR TIMINGS");
    std::process::exit(43);
}
RUST
printf '[package]\nname="ablation-control-fixture"\nversion="0.0.0"\nedition="2024"\n' > "$repo/Cargo.toml"
git -C "$repo" init -q
git -C "$repo" add .
git -C "$repo" -c user.name='Control fixture' -c user.email='fixture@example.invalid' commit -qm 'Explicit no-timing controller fixture'
commit=$(git -C "$repo" rev-parse HEAD)
rustc --edition=2024 -Ctarget-feature=+crt-static -Cdebuginfo=1 "$repo/src/main.rs" -o "$bundle/ocr-profiling"
# The stub stands in for every executable that bundle admission requires.
mapfile -t executables < <(
    # shellcheck source=scripts/profile-native.sh
    source "$root/scripts/profile-native.sh"
    printf '%s\n' "${bundle_executables[@]}"
)
[[ ${#executables[@]} -gt 1 && ${executables[0]} = ocr-profiling ]]
for target in "${executables[@]:1}"; do cp "$bundle/ocr-profiling" "$bundle/$target"; done
cp "$root/backend/tests/data/two-pages.tiff" "$root/runtime/crates/indra-engine/tests/data/math.png" "$root/runtime/crates/indra-engine/tests/data/two-pages.expected.txt" "$root/runtime/crates/indra-engine/tests/data/math-page.expected.txt" "$bundle/fixtures/"
cp "$repo/runtime/validation/native-profile-protocol.json" "$repo/runtime/validation/native-ablation-protocol.json" "$bundle/"
git -C "$repo" archive --format=tar.gz HEAD > "$bundle/relink/rust-source.tar.gz"
printf 'source_commit=%s\ncargo_profile=profiling\nlto=thin\n' "$commit" > "$bundle/build-evidence/execution-source.txt"
printf 'runtime_kind=native\nbinary_sha256=%s\n' "$(sha256sum "$bundle/ocr-profiling"|cut -d' ' -f1)" > "$bundle/relink/build-identity.txt"
(cd "$bundle/relink" && find . -type f ! -path ./SHA256SUMS -print0 | sort -z | xargs -0 sha256sum > SHA256SUMS)
(cd "$bundle" && find . -type f ! -path ./artifacts.sha256 -print0 | sort -z | xargs -0 sha256sum > artifacts.sha256)
mapfile -t available < <(bash "$root/scripts/profile-native.sh" --available-cpus)
# MILLICPUS CPUSET THREADS CV_THREADS ITERATIONS: every quota and thread shape the studies use.
for shape in "1000 ${available[0]} 1 1 32" "2000 ${available[0]},${available[1]} 2 2 32" "1500 ${available[0]},${available[1]} 2 1 64"; do
    read -r millicpus cpuset threads cv iterations <<< "$shape"
    status=0
    bash "$repo/scripts/profile-native-docker.sh" --ablation-cell "$bundle" "$temporary/models" "$temporary/cell-$millicpus" "$image" \
        "$millicpus" "$cpuset" math 8 "$threads" "$cv" false thin "$iterations" \
        > "$temporary/run-$millicpus.stdout" 2> "$temporary/run-$millicpus.stderr" || status=$?
    [[ $status != 0 && ! -e $temporary/cell-$millicpus/completion.json ]]
    phase=$temporary/cell-$millicpus/cell/ablation
    [[ $(cat "$phase/recognition.exit") = 43 ]]
    grep -Fq 'EXPECTED CONTROL FIXTURE EXIT 43: NO OCR OR TIMINGS' "$phase/recognition.stderr.log"
    grep -Fq -- "--threads $threads --cv-threads $cv " "$phase/recognition.command.txt"
    [[ $(cat "$phase/before/cpu.max") = "$((millicpus * 100)) 100000" ]]
    jq -e '.exit==43 and .resource_capture_exit==0' "$phase/phase-status.json" > /dev/null
    jq -e '.exit!=0 and .cleanup_exit==0' "$temporary/cell-$millicpus/status.json" > /dev/null
done
status=0
bash "$repo/scripts/profile-native-docker.sh" --ablation-cell "$bundle" "$temporary/models" "$temporary/empty-journal" "$image" \
    1000 "${available[0]}" tiff 1 1 1 false thin 32 > "$temporary/empty-journal.stdout" 2> "$temporary/empty-journal.stderr" || status=$?
[[ $status != 0 && ! -e $temporary/empty-journal/completion.json ]]
phase=$temporary/empty-journal/cell/ablation
[[ $(cat "$phase/recognition.exit") = 0 && -s $phase/loop-enable/threads.tsv && -s $phase/loop-disable/cpu.stat ]]
grep -Fq 'invalid ablation request journal' "$phase/container.stderr"
if grep -Fq 'jq: error:' "$phase/container.stderr"; then exit 1; fi
jq -e '.exit!=0 and .cleanup_exit==0' "$temporary/empty-journal/status.json" > /dev/null
echo 'Real Docker 1, 1.5 and 2-CPU invocations, the counter exchange, failure retention and owned cleanup fixtures passed; no OCR timings produced'
