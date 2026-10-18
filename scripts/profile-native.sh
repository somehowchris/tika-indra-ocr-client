#!/usr/bin/env bash
# Diagnostic-only Linux profiling; every measured child shares its phase cgroup.
set -euo pipefail
export LC_ALL=C
# Native tests exercise the same bounded stack as the production worker.
export RUST_MIN_STACK=8388608 LANG=C TZ=UTC DEBUGINFOD_URLS=
export OMP_NUM_THREADS=1 OMP_THREAD_LIMIT=1 OPENBLAS_NUM_THREADS=1 MKL_NUM_THREADS=1 RAYON_NUM_THREADS=1
shopt -s nullglob
fail() { printf 'native profiling failed: %s\n' "$*" >&2; exit 1; }
bounded() { timeout --signal=TERM --kill-after=10s "$@"; }
script=$(readlink -e -- "${BASH_SOURCE[0]}")
root=$(dirname "$(dirname "$script")")

expand_cpus() {
    local source=$1 part first last value
    local -a parts
    IFS=, read -r -a parts <<< "$source"
    [[ ${#parts[@]} -gt 0 ]] || return 1
    for part in "${parts[@]}"; do
        if [[ $part =~ ^([0-9]+)-([0-9]+)$ ]]; then
            first=${BASH_REMATCH[1]}; last=${BASH_REMATCH[2]}
        elif [[ $part =~ ^[0-9]+$ ]]; then
            first=$part; last=$part
        else return 1; fi
        [[ ${#first} -le 7 && ${#last} -le 7 ]] || return 1
        first=$((10#$first)); last=$((10#$last))
        ((first <= last && last < 1048576)) || return 1
        for ((value=first; value<=last; value++)); do printf '%s\n' "$value"; done
    done
}

logged() {
    local name=$1 seconds=$2 status=0
    shift 2
    printf '%q ' "$@" > "$phase_out/$name.command.txt"
    printf '\n' >> "$phase_out/$name.command.txt"
    bounded "${seconds}s" "$@" > "$phase_out/$name.stdout.log" 2> "$phase_out/$name.stderr.log" || status=$?
    printf '%s\n' "$status" > "$phase_out/$name.exit"
    if [[ $status != 0 ]]; then
        printf 'profile command failed: %s, exit=%s; bounded command/stdout/stderr follow\n' "$name" "$status" >&2
        tail -c 4096 "$phase_out/$name.command.txt" >&2
        tail -c 16384 "$phase_out/$name.stdout.log" >&2
        tail -c 16384 "$phase_out/$name.stderr.log" >&2
    fi
    return "$status"
}

snapshot() {
    local destination=$1 name
    mkdir -p -- "$destination" || return
    for name in cpu.max cpu.stat cpuset.cpus.effective memory.max memory.swap.max memory.current memory.peak memory.events pids.max pids.current; do
        cat "$phase_group/$name" > "$destination/$name" || return
    done
    cat /proc/self/cgroup > "$destination/process.cgroup" || return
    cat "/proc/$$/status" > "$destination/process.status" || return
    cat "/proc/$$/limits" > "$destination/process.limits" || return
    ulimit -S -s > "$destination/stack-soft-kib" || return
}

finish_phase() {
    local status=$? capture=0 before after
    trap - EXIT
    snapshot "$phase_out/after" || capture=$?
    if [[ $capture = 0 ]]; then
        before=$(awk '$1 == "oom_kill" {print $2}' "$phase_out/before/memory.events")
        after=$(awk '$1 == "oom_kill" {print $2}' "$phase_out/after/memory.events")
        if [[ ! $before =~ ^[0-9]+$ || ! $after =~ ^[0-9]+$ || $before != "$after" ]]; then capture=1; fi
    fi
    if [[ $status = 0 && $capture != 0 ]]; then status=$capture; fi
    jq -n --argjson exit "$status" --argjson capture "$capture" \
        '{exit:$exit,resource_capture_exit:$capture}' > "$phase_out/phase-status.json" || status=1
    exit "$status"
}

attest() {
    local cpus=$1 affinity=$2 unit=$3 relative quota period extra current memory depth=0
    relative=$(awk -F: '$1 == "0" {print $3}' /proc/self/cgroup)
    if [[ $unit = docker ]]; then
        # The local Docker controller supplies a private cgroup namespace.
        # Its root is the exact limited container, not the host root cgroup.
        [[ $relative = / ]] || fail 'Docker phase requires its private cgroup namespace'
    else
        [[ $relative = /* && $(basename "$relative") = "$unit" ]] || fail 'wrong phase cgroup'
    fi
    phase_group=$(readlink -e "/sys/fs/cgroup$relative")
    [[ ( $phase_group = /sys/fs/cgroup/* || ( $unit = docker && $phase_group = /sys/fs/cgroup ) ) && -f $phase_group/cpu.max ]] \
        || fail 'cgroup v2 is required'
    grep -Fxq "$$" "$phase_group/cgroup.procs" || fail 'phase shell is outside its owned cgroup'
    read -r quota period extra < "$phase_group/cpu.max"
    [[ $quota = "$((cpus * 100000))" && $period = 100000 && -z $extra ]] || fail 'actual CPU quota differs'
    [[ $(cat "$phase_group/memory.max") = 3221225472 ]] || fail 'actual memory limit differs'
    [[ $(cat "$phase_group/memory.swap.max") = 0 ]] || fail 'swap must be disabled'
    [[ $(cat "$phase_group/pids.max") = 128 ]] || fail 'actual task limit differs'
    [[ $(ulimit -S -s) = 8192 ]] || fail 'main-thread stack must reserve 8 MiB'
    expand_cpus "$affinity" > "$phase_out/expected-cpus.txt"
    expand_cpus "$(cat "$phase_group/cpuset.cpus.effective")" > "$phase_out/cgroup-cpus.txt"
    expand_cpus "$(awk '/^Cpus_allowed_list:/ {print $2}' "/proc/$$/status")" > "$phase_out/process-cpus.txt"
    cmp "$phase_out/expected-cpus.txt" "$phase_out/cgroup-cpus.txt" || fail 'effective cpuset differs'
    cmp "$phase_out/expected-cpus.txt" "$phase_out/process-cpus.txt" || fail 'actual scheduler affinity differs'
    [[ $(wc -l < "$phase_out/expected-cpus.txt") = "$cpus" ]] || fail 'affinity cardinality differs'
    current=$phase_group
    : > "$phase_out/ancestors.txt"
    while :; do
        depth=$((depth + 1))
        ((depth <= 64)) || fail 'unbounded cgroup ancestry'
        printf '%s\n' "$current" >> "$phase_out/ancestors.txt"
        if [[ -f $current/cpu.max ]]; then
            read -r quota period extra < "$current/cpu.max"
            printf 'cpu.max %s %s\n' "$quota" "$period" >> "$phase_out/ancestors.txt"
            [[ $period =~ ^[0-9]+$ && -z $extra ]] || fail 'invalid ancestor CPU period'
            if [[ $quota != max ]]; then
                [[ $quota =~ ^[0-9]+$ ]] || fail 'invalid ancestor CPU quota'
                ((quota >= cpus * period)) || fail 'an ancestor has a tighter CPU quota'
            fi
        fi
        if [[ -f $current/memory.max ]]; then
            memory=$(cat "$current/memory.max")
            printf 'memory.max %s\n' "$memory" >> "$phase_out/ancestors.txt"
            if [[ $memory != max ]]; then
                [[ $memory =~ ^[0-9]+$ ]] || fail 'invalid ancestor memory limit'
                ((memory >= 3221225472)) || fail 'an ancestor has a tighter memory limit'
            fi
        fi
        [[ $current = /sys/fs/cgroup ]] && break
        current=$(dirname "$current")
        [[ $current = /sys/fs/cgroup || $current = /sys/fs/cgroup/* ]] || fail 'invalid cgroup ancestry'
    done
    snapshot "$phase_out/before"
    trap finish_phase EXIT
}

validate_stats() {
    local file=$1 cpus=$2 math=$3 frames=$4 bytes=$5
    jq -se --argjson cpus "$cpus" --argjson math "$math" --argjson frames "$frames" --argjson bytes "$bytes" '
        length == 64 and map(.iteration) == [range(0;64)] and
        all(.[]; .diagnostic == false and .threads == $cpus and .cv_threads == $cpus
            and .math == $math and .batch_size == 64 and .frames == $frames
            and .output_bytes == $bytes and .intra_op_spinning == true and .inter_op_spinning == true
            and (.elapsed_us | type == "number" and isfinite and . > 0)
            and (.stage_us | type == "array" and length == 7
                and all(.[]; type == "number" and isfinite and . >= 0)))
        ' "$file" > /dev/null || fail 'request statistics violate the profile protocol'
}

ocr_profile() {
    local bundle=$1 models=$2 cpus=$3 fixture=$4 input expected frames bytes math block
    local -a command math_args=()
    if [[ $fixture = tiff ]]; then
        input=$bundle/fixtures/two-pages.tiff; expected=$bundle/fixtures/two-pages.expected.txt
        frames=2; bytes=33; math=false
    else
        input=$bundle/fixtures/math.png; expected=$bundle/fixtures/math-page.expected.txt
        frames=1; bytes=293; math=true; math_args=(--math)
    fi
    command=("$bundle/ocr-profiling" "$input" --model-dir "$models"
        --threads "$cpus" --cv-threads "$cpus" --batch-size 64
        --intra-op-spinning true --inter-op-spinning true --warmup
        --iterations 64 --timeout-seconds 60 --expect "$expected" "${math_args[@]}")
    for block in unrecorded-before recorded unrecorded-after; do
        if [[ $block = recorded ]]; then
            mkfifo "$phase_out/control.fifo" "$phase_out/ack.fifo"
            logged "$block" 600 "$PERF" record --no-buildid-cache --delay=-1 \
                --event cpu-clock:u --freq 99 --strict-freq \
                --call-graph dwarf,16384 --mmap-pages=32M --max-size=256M \
                --output "$phase_out/perf.data" \
                --control="fifo:$phase_out/control.fifo,$phase_out/ack.fifo" \
                -- "${command[@]}" --perf-control "$phase_out/control.fifo" \
                --perf-ack "$phase_out/ack.fifo" --output "$phase_out/$block.txt" \
                --stats "$phase_out/$block.jsonl"
        else
            logged "$block" 600 "${command[@]}" --output "$phase_out/$block.txt" \
                --stats "$phase_out/$block.jsonl"
        fi
        validate_stats "$phase_out/$block.jsonl" "$cpus" "$math" "$frames" "$bytes"
        cmp "$expected" "$phase_out/$block.txt" || fail 'final fixture text differs'
    done
    logged perf-report 180 "$PERF" report --stdio --header --show-nr-samples --no-children \
        --percent-limit 0 --sort dso,symbol --input "$phase_out/perf.data"
    logged perf-script 180 "$PERF" script --header --show-lost-events \
        -F comm,pid,tid,time,event,ip,sym,dso --input "$phase_out/perf.data"
    cp "$phase_out/perf-report.stdout.log" "$phase_out/perf-report.txt"
    cp "$phase_out/perf-script.stdout.log" "$phase_out/perf-script.txt"
    # Inferno 0.12.8 splits the perf event at the first colon: its filter
    # is cpu-clock, while the strict raw-stream admission still requires :u.
    logged collapse 180 "$INFERNO_COLLAPSE" --addrs --nthreads 1 --event-filter cpu-clock \
        "$phase_out/perf-script.txt"
    cp "$phase_out/collapse.stdout.log" "$phase_out/folded.txt"
    logged flamegraph 180 "$INFERNO_FLAMEGRAPH" --deterministic --minwidth 0 --width 1600 \
        --title "Native Indra $fixture: $cpus CPU, warmed loop" \
        --subtitle 'Software CPU-time samples; synthetic implementation fixture' \
        --notes 'All samples retained. Stats/control overhead included. Native source lines may be absent.' \
        "$phase_out/folded.txt"
    cp "$phase_out/flamegraph.stdout.log" "$phase_out/flamegraph.svg"
    logged render 180 rsvg-convert --output "$phase_out/flamegraph.png" "$phase_out/flamegraph.svg"
    [[ -s $phase_out/flamegraph.svg && -s $phase_out/flamegraph.png ]] || fail 'flamegraph is missing'
    logged sample-admission 180 /bin/bash "$root/scripts/verify-native-profile-samples.sh" \
        "$phase_out/perf-report.txt" "$phase_out/perf-script.txt" "$phase_out/recorded.jsonl" \
        "$phase_out/folded.txt" "$phase_out/sample-quality.json"
    jq -n --slurpfile before "$phase_out/unrecorded-before.jsonl" \
        --slurpfile during "$phase_out/recorded.jsonl" --slurpfile after "$phase_out/unrecorded-after.jsonl" '
        def quantiles: map(.elapsed_us) | sort | . as $x |
            {count:length,p50_us:$x[((length*0.50|ceil)-1)],
             p95_us:$x[((length*0.95|ceil)-1)],p99_us:$x[((length*0.99|ceil)-1)]};
        ($before|quantiles) as $b | ($during|quantiles) as $d | ($after|quantiles) as $a |
        {scope:"descriptive successful warmed requests; no independent-sample or causal interval",
         before:$b,recorded:$d,after:$a,
         recorded_to_before_median_ratio:($d.p50_us/$b.p50_us),
         recorded_to_after_median_ratio:($d.p50_us/$a.p50_us)}' > "$phase_out/latency-summary.json"
}

criterion_profile() {
    local bundle=$1 name directory count=0
    for name in postprocessing control_path verification propagation; do
        export CRITERION_HOME="$phase_out/criterion/$name"
        logged "$name" 600 "$bundle/$name" --bench --noplot \
            --warm-up-time 1 --measurement-time 3 --sample-size 100
    done
    find "$phase_out/criterion" -type d -name new -print0 > "$phase_out/criterion-directories.nul"
    : > "$phase_out/criterion-results.jsonl"
    while IFS= read -r -d '' directory; do
        jq -e '.iters as $i | .times as $t |
            ($i|type=="array" and length==100 and all(.[]; type=="number" and isfinite and .>0)) and
            ($t|type=="array" and length==100 and all(.[]; type=="number" and isfinite and .>0))' \
            "$directory/sample.json" > /dev/null || fail 'Criterion samples are incomplete'
        jq -e '.mean | [.point_estimate,.confidence_interval.lower_bound,.confidence_interval.upper_bound] |
            all(.[]; type=="number" and isfinite and .>0)' \
            "$directory/estimates.json" > /dev/null || fail 'invalid Criterion estimates'
        jq -nc --slurpfile benchmark "$directory/benchmark.json" \
            --slurpfile estimates "$directory/estimates.json" \
            '{benchmark:$benchmark[0],estimates:$estimates[0]}' >> "$phase_out/criterion-results.jsonl"
        count=$((count + 1))
    done < "$phase_out/criterion-directories.nul"
    [[ $count = 46 ]] || fail 'Criterion case count differs'
    jq -se --slurpfile protocol "$bundle/native-profile-protocol.json" '
        ($protocol[0].criterion) as $p |
        ([ $p.planner_policies[] as $policy | $p.planner_batch_caps[] as $cap |
            $p.planner_counts[] as $count |
            "recognition_width_plan/\($policy)-b\($cap)/\($count)" ] +
         [ $p.scan_arms[] as $arm | $p.scan_shapes[] as $shape |
            "recognition_tensor_width_scan/\($arm)/\($shape)" ] +
         $p.additional_cases | sort) as $expected |
        (map(.benchmark.full_id)|sort) == $expected' \
        "$phase_out/criterion-results.jsonl" > /dev/null || fail 'Criterion case identities differ'
    jq -s '
        map(select(.benchmark.group_id=="recognition_tensor_width_scan")) |
        group_by(.benchmark.value_str) | map(
            (map(select(.benchmark.function_id=="legacy_max"))[0]) as $old |
            (map(select(.benchmark.function_id=="proved_plan"))[0]) as $new |
            {shape:$old.benchmark.value_str,legacy_mean:$old.estimates.mean,proved_mean:$new.estimates.mean,
             descriptive_point_mean_ratio:($new.estimates.mean.point_estimate/$old.estimates.mean.point_estimate)})' \
        "$phase_out/criterion-results.jsonl" > "$phase_out/tensor-scan-comparison.json"
    jq -e 'length==8 and all(.[]; .descriptive_point_mean_ratio|type=="number" and isfinite and .>0)' \
        "$phase_out/tensor-scan-comparison.json" > /dev/null || fail 'incomplete scan comparison'
}

phase() {
    local bundle=$1 models=$2 cpus=$3 affinity=$4 unit=$5 name=$6
    phase_out=$7
    [[ $cpus = 1 || $cpus = 2 ]] || fail 'CPU count must be one or two'
    [[ $name = gates || $name = criterion || $name = tiff || $name = math ]] || fail 'unknown phase'
    ulimit -f 1048576
    ulimit -S -s 8192
    attest "$cpus" "$affinity" "$unit"
    if [[ $unit = docker ]]; then
        # The host checks the actual Docker leaf and otherwise-hidden ancestors
        # before any model/test/benchmark work starts in this private namespace.
        local admission_started=$SECONDS
        while [[ ! -s $phase_out/host-admission.json ]]; do
            ((SECONDS-admission_started<30)) || fail 'host cgroup admission was not confirmed'
            sleep 0.05
        done
        jq -e '.admitted==true' "$phase_out/host-admission.json" > /dev/null || fail 'host cgroup admission failed'
        [[ $PERF = /usr/bin/perf && $INFERNO_COLLAPSE = /profile-tools/bin/inferno-collapse-perf \
            && $INFERNO_FLAMEGRAPH = /profile-tools/bin/inferno-flamegraph ]] || fail 'profiling tool paths differ from the image contract'
        [[ -f /profile-tools/build-identities.sha256 ]] || fail 'profiling tool identity manifest is missing'
        (cd / && sha256sum --check --strict /profile-tools/build-identities.sha256) > "$phase_out/tool-verification.txt"
        cmp /profile-tools/build-recipe.Dockerfile "$root/docker/profile-tools.Dockerfile" || fail 'profiling tool recipe differs from execution source'
        cp /profile-tools/packages.tsv "$phase_out/tools-packages.tsv"
        cp /profile-tools/build-identities.sha256 /profile-tools/inferno-source.txt "$phase_out/"
        sha256sum "$PERF" "$INFERNO_COLLAPSE" "$INFERNO_FLAMEGRAPH" /usr/bin/rsvg-convert \
            "$script" "$root/scripts/verify-native-profile-samples.sh" > "$phase_out/execution-identities.sha256"
        "$PERF" version --build-options > "$phase_out/perf-version.txt"
    fi
    export INDRA_TEST_MODEL_DIR="$models" INDRA_TEST_THREADS="$cpus"
    if [[ $name = gates ]]; then
        logged unit 600 "$bundle/gates/unit" --include-ignored --test-threads=1 --nocapture
        logged native 180 "$bundle/gates/native" --ignored --test-threads=1 --nocapture
        logged allocations 60 "$bundle/gates/allocations"
        logged perf-control 60 "$bundle/gates/perf-control" --test-threads=1 --nocapture
        logged ocr-cli 60 "$bundle/gates/ocr-cli" --test-threads=1 --nocapture
        logged core-allocations 60 "$bundle/gates/core-allocations"
        logged telemetry-allocations 60 "$bundle/gates/telemetry-allocations"
        logged model-allocations 120 "$bundle/gates/model-allocations"
        jq -se 'length==2 and map(.profile)==["text","math"] and all(.[]; .iterations==100 and
            ([.allocations,.reallocations,.deallocations,.bytes_allocated,.bytes_deallocated] |
             all(.[]; type=="number" and isfinite and .>=0)) and
             (.bytes_reallocated|type=="number" and isfinite and floor==.))' \
            "$phase_out/model-allocations.stdout.log" > /dev/null || fail 'model allocation observations are invalid'
    elif [[ $name = criterion ]]; then
        criterion_profile "$bundle"
    else
        ocr_profile "$bundle" "$models" "$cpus" "$name"
    fi
}

# Both controllers admit exactly the same immutable bundle before any phase.
verify_manifest() {
    local directory=$1 manifest=$2 output=$3 label=$4 line relative
    [[ -f $directory/$manifest && ! -L $directory/$manifest ]] || fail "$label manifest is missing"
    [[ -z $(find "$directory" -type l -print -quit) ]] || fail "$label contains a symlink"
    : > "$output/$label-manifest.paths"
    while IFS= read -r line; do
        [[ $line =~ ^[a-f0-9]{64}\ \ [A-Za-z0-9_./+~:@-]+$ ]] || fail "$label manifest record is invalid"
        relative=${line:66}; relative=${relative#./}
        [[ $relative != /* && /$relative/ != *'/../'* && /$relative/ != *'/./'* && $relative != *'//'* && $relative != "$manifest" ]] \
            || fail "$label manifest path escapes or repeats itself"
        [[ -f $directory/$relative && ! -L $directory/$relative ]] || fail "$label manifest file is missing"
        printf '%s\n' "$relative" >> "$output/$label-manifest.paths"
    done < "$directory/$manifest"
    sort "$output/$label-manifest.paths" > "$output/$label-manifest.sorted"
    [[ ! -s $output/$label-manifest.sorted || -z $(uniq -d "$output/$label-manifest.sorted") ]] || fail "$label manifest has duplicates"
    find "$directory" -type f ! -path "$directory/$manifest" -printf '%P\n' | sort > "$output/$label-files.sorted"
    cmp "$output/$label-manifest.sorted" "$output/$label-files.sorted" || fail "$label manifest omits or adds files"
    (cd "$directory" && sha256sum --check --strict "$manifest") > "$output/$label-verification.txt" || fail "$label manifest verification failed"
}

verify_source_closure() (
    local bundle=$1 output=$2 commit temporary relative
    [[ -f $bundle/build-evidence/execution-source.txt ]] || fail 'execution source identity is missing'
    commit=$(sed -n 's/^source_commit=//p' "$bundle/build-evidence/execution-source.txt")
    [[ $commit =~ ^[a-f0-9]{40}$ && $(git -C "$root" rev-parse HEAD) = "$commit" ]] || fail 'bundle does not name the checked-out source commit'
    grep -Fxq 'cargo_profile=profiling' "$bundle/build-evidence/execution-source.txt" || fail 'bundle was not built with the profiling profile'
    git -C "$root" ls-files -- Cargo.toml Cargo.lock src tests benches runtime/Cargo.toml runtime/Cargo.lock runtime/validation/native-profile-protocol.json runtime/crates shared scripts docker \
        | awk '/[.]rs$|Cargo[.](toml|lock)$|^scripts\/|^docker\/|^runtime\/validation\/native-profile-protocol[.]json$/' > "$output/source-closure.paths"
    [[ -s $output/source-closure.paths ]] || fail 'source closure is empty'
    git -C "$root" ls-files --others --exclude-standard -- Cargo.toml Cargo.lock src tests benches runtime/Cargo.toml runtime/Cargo.lock runtime/validation/native-profile-protocol.json runtime/crates shared scripts docker \
        | awk '/[.]rs$|Cargo[.](toml|lock)$|^scripts\/|^docker\/|^runtime\/validation\/native-profile-protocol[.]json$/' > "$output/untracked-execution-source.paths"
    [[ ! -s $output/untracked-execution-source.paths ]] || fail 'compiled or execution source is untracked'
    while IFS= read -r relative; do
        [[ $relative =~ ^[A-Za-z0-9_./-]+$ && -f $root/$relative && ! -L $root/$relative ]] || fail 'source closure contains an unsafe path'
        git -C "$root" diff --quiet "$commit" -- "$relative" || fail 'compiled or execution source differs from the named commit'
    done < "$output/source-closure.paths"
    # Only explicitly selected regular source paths are extracted. No directory
    # entry or archive-supplied parent symlink is selected for extraction.
    temporary=$(mktemp -d "$output/.source-admission.XXXXXXXX")
    trap 'rm -rf -- "$temporary"' EXIT
    tar -tzf "$bundle/relink/rust-source.tar.gz" | sort > "$output/source-archive.paths"
    [[ -z $(uniq -d "$output/source-archive.paths") ]] || fail 'source archive has duplicate members'
    tar -xzf "$bundle/relink/rust-source.tar.gz" -C "$temporary" --no-same-owner --no-same-permissions \
        --verbatim-files-from -T "$output/source-closure.paths"
    while IFS= read -r relative; do
        [[ -f $temporary/$relative && ! -L $temporary/$relative ]] || fail 'archive source is not a regular file'
        cmp "$root/$relative" "$temporary/$relative" || fail 'relink source differs from the checked-out compiled/execution source'
    done < "$output/source-closure.paths"
    (cd "$temporary" && while IFS= read -r relative; do sha256sum "$relative"; done < "$output/source-closure.paths") \
        > "$output/source-closure.sha256"
)

verify_bundle() {
    local bundle=$1 output=$2 ocr_hash binary name
    cmp "$bundle/native-profile-protocol.json" "$root/runtime/validation/native-profile-protocol.json" \
        || fail 'artifact protocol differs from checked-out execution source'
    verify_manifest "$bundle" artifacts.sha256 "$output" bundle
    verify_manifest "$bundle/relink" SHA256SUMS "$output" relink
    ocr_hash=$(sha256sum "$bundle/ocr-profiling" | cut -d' ' -f1)
    grep -Fxq "binary_sha256=$ocr_hash" "$bundle/relink/build-identity.txt" \
        || fail 'relink materials do not identify this profiling executable'
    grep -Fxq 'runtime_kind=native' "$bundle/relink/build-identity.txt" || fail 'native relink material identity is missing'
    jq -r '.fixtures[] | .input_sha256+"  "+(.input|split("/")|last),
        .expected_sha256+"  "+(.expected|split("/")|last)' \
        "$bundle/native-profile-protocol.json" > "$output/fixture-sha256.txt"
    (cd "$bundle/fixtures" && sha256sum --check --strict "$output/fixture-sha256.txt") \
        > "$output/fixture-verification.txt" || fail 'fixtures do not match the prespecified hashes'
    for binary in ocr-profiling engine-acceptance postprocessing control_path verification propagation \
        gates/unit gates/native gates/allocations gates/perf-control gates/ocr-cli \
        gates/core-allocations gates/telemetry-allocations gates/model-allocations; do
        [[ -f $bundle/$binary && -x $bundle/$binary && ! -L $bundle/$binary ]] || fail "required executable is missing: $binary"
        name=${binary//\//-}
        readelf -W -l "$bundle/$binary" > "$output/$name.program-headers.txt"
        readelf -W -d "$bundle/$binary" > "$output/$name.dynamic.txt"
        if grep -q INTERP "$output/$name.program-headers.txt" || grep -q NEEDED "$output/$name.dynamic.txt"; then
            fail "executable is not static: $binary"
        fi
    done
    readelf -W -S "$bundle/ocr-profiling" > "$output/ocr-profiling.sections.txt"
    grep -q '[.]debug_line' "$output/ocr-profiling.sections.txt" || fail 'profiling executable has no DWARF line tables'
    verify_source_closure "$bundle" "$output"
    cp "$bundle/native-profile-protocol.json" "$output/protocol.json"
}

if [[ ${1:-} = --available-cpus ]]; then
    [[ $# = 1 ]] || fail 'invalid CPU query arguments'
    expand_cpus "$(awk '/^Cpus_allowed_list:/ {print $2}' "/proc/$$/status")"
    exit 0
fi

if [[ ${1:-} = --verify-bundle ]]; then
    [[ $# = 3 && -d $2 && -d $3 ]] || fail 'invalid bundle admission arguments'
    verify_bundle "$(readlink -e "$2")" "$(readlink -e "$3")"
    exit 0
fi

if [[ ${1:-} = --phase ]]; then
    shift
    [[ $# = 7 ]] || fail 'invalid internal phase arguments'
    phase "$@"
    exit 0
fi

[[ $# = 3 ]] || fail 'usage: profile-native.sh BUNDLE MODEL_CACHE NEW_OUTPUT_DIRECTORY'
[[ $EUID = 0 ]] || fail 'run under sudo so systemd can enforce and attest total phase limits'
bundle=$(readlink -e -- "$1")
models=$(readlink -e -- "$2")
[[ -d $bundle && -d $models ]] || fail 'existing bundle and model directories required'
[[ ! -e $3 && ! -L $3 ]] || fail 'refusing to reuse a prior measurement directory'
mkdir -p -- "$(dirname "$3")"
mkdir -- "$3"
output=$(readlink -e -- "$3")
for tool in systemd-run systemctl timeout jq sha256sum rsvg-convert lscpu taskset readelf; do
    command -v "$tool" > /dev/null || fail "required tool missing: $tool"
done
for tool in PERF INFERNO_COLLAPSE INFERNO_FLAMEGRAPH; do
    [[ ${!tool:-} = /* && -f ${!tool} && -x ${!tool} ]] || fail "absolute executable required: $tool"
done
verify_bundle "$bundle" "$output"
sha256sum "$script" "$root/scripts/verify-native-profile-samples.sh" "$bundle/native-profile-protocol.json" "$PERF" "$INFERNO_COLLAPSE" "$INFERNO_FLAMEGRAPH" \
    > "$output/execution-identities.sha256"
bounded 30s "$PERF" version --build-options > "$output/perf-version.txt"
lscpu --json > "$output/lscpu.json"
uname -a > "$output/kernel.txt"
cat /proc/cpuinfo > "$output/cpuinfo.txt"
for name in perf_event_paranoid perf_event_max_sample_rate perf_event_mlock_kb kptr_restrict; do
    cat "/proc/sys/kernel/$name" > "$output/$name"
done
expand_cpus "$(awk '/^Cpus_allowed_list:/ {print $2}' "/proc/$$/status")" > "$output/available-cpus.txt"
mapfile -t available < "$output/available-cpus.txt"
[[ ${#available[@]} -ge 2 ]] || fail 'two schedulable CPUs are required'
run_id="indra-profile-$$-$RANDOM"
active_unit=

finish() {
    local status=$? cleanup=0 load description stopped
    trap - EXIT
    if [[ -n $active_unit ]]; then
        if bounded 15s systemctl show "$active_unit" --property=LoadState --property=Description \
            > "$output/cleanup-unit.txt" 2> "$output/cleanup-unit.stderr.log"; then
            load=$(sed -n 's/^LoadState=//p' "$output/cleanup-unit.txt")
            description=$(sed -n 's/^Description=//p' "$output/cleanup-unit.txt")
            if [[ $load = not-found ]]; then
                printf 'owned unit already unloaded\n' > "$output/cleanup.txt"
            elif [[ $description = "$run_id" ]]; then
                stopped=0
                bounded 30s systemctl stop "$active_unit" > "$output/cleanup.txt" \
                    2> "$output/cleanup.stderr.log" || stopped=$?
                printf '%s\n' "$stopped" > "$output/cleanup.exit"
                [[ $stopped = 0 ]] || cleanup=1
            else
                printf 'refused cleanup: unit ownership could not be confirmed\n' > "$output/cleanup.txt"
                cleanup=1
            fi
        else cleanup=1; fi
    fi
    if [[ $status = 0 && $cleanup != 0 ]]; then status=$cleanup; fi
    jq -n --argjson exit "$status" --argjson cleanup "$cleanup" \
        '{exit:$exit,cleanup_exit:$cleanup,visual_review:"pending"}' > "$output/status.json" || status=1
    exit "$status"
}
trap finish EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
for cpus in 1 2; do
    affinity=${available[0]}
    if [[ $cpus = 2 ]]; then affinity="$affinity,${available[1]}"; fi
    mkdir "$output/cpu-$cpus"
    for name in gates criterion tiff math; do
        destination="$output/cpu-$cpus/$name"
        mkdir "$destination"
        active_unit="$run_id-$cpus-$name.service"
        [[ $(bounded 15s systemctl show "$active_unit" --property=LoadState --value) = not-found ]] \
            || fail 'refusing a pre-existing systemd unit'
        status=0
        bounded 930s systemd-run --unit="$active_unit" --description="$run_id" --wait --pipe --collect \
            --property="CPUQuota=$((cpus * 100))%" --property=CPUQuotaPeriodSec=100ms \
            --property="AllowedCPUs=$affinity" --property="CPUAffinity=${affinity//,/ }" \
            --property=MemoryMax=3G --property=MemorySwapMax=0 --property=TasksMax=128 \
            --property=CPUAccounting=yes --property=MemoryAccounting=yes \
            --property=RuntimeMaxSec=900 --property=TimeoutStopSec=15 --property=KillMode=control-group \
            --property=NoNewPrivileges=yes --property=ProtectControlGroups=yes --property=PrivateNetwork=yes \
            --setenv="PERF=$PERF" --setenv="INFERNO_COLLAPSE=$INFERNO_COLLAPSE" \
            --setenv="INFERNO_FLAMEGRAPH=$INFERNO_FLAMEGRAPH" \
            /bin/bash "$script" --phase "$bundle" "$models" "$cpus" "$affinity" "$active_unit" "$name" "$destination" \
            > "$destination/systemd.stdout.log" 2> "$destination/systemd.stderr.log" || status=$?
        printf '%s\n' "$status" > "$destination/systemd.exit"
        if [[ $status != 0 ]]; then
            printf 'profile phase failed: %s/%s, exit=%s; bounded retained diagnostics follow\n' "$cpus" "$name" "$status" >&2
            tail -c 32768 "$destination/systemd.stdout.log" >&2
            tail -c 32768 "$destination/systemd.stderr.log" >&2
            if [[ -f $destination/phase-status.json ]]; then
                tail -c 4096 "$destination/phase-status.json" >&2
            fi
            fail "phase $cpus/$name failed; raw evidence was retained"
        fi
        jq -e '.exit==0 and .resource_capture_exit==0' "$destination/phase-status.json" > /dev/null \
            || fail 'phase terminal/resource evidence is incomplete'
        active_unit=
    done
done
jq -n '{completed:true,cpus:[1,2],phases_per_cpu:4,visual_review:"pending"}' > "$output/completion.json"
