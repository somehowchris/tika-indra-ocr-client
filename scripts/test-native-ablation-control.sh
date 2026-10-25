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
if printf '[]\n' | jq -f "$root/scripts/summarize-native-ablations.jq" > "$temporary/empty.json" 2> "$temporary/empty.stderr"; then exit 1; fi
[[ ${1:-} = --docker ]] || { [[ $# = 0 ]]; echo 'Offline ablation schedule and missing-data rejection passed'; exit 0; }
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
for target in engine-acceptance postprocessing control_path verification propagation gates/unit gates/native gates/allocations gates/perf-control gates/ocr-cli gates/core-allocations gates/telemetry-allocations gates/model-allocations; do cp "$bundle/ocr-profiling" "$bundle/$target"; done
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
