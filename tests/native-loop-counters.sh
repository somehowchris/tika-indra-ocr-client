#!/usr/bin/env bash
# The warmed-loop counter exchange of scripts/profile-native.sh against a stand-in
# perf-control client on real Linux FIFOs and /proc; no OCR, models or timings.
# Needs Linux, bash, rustc and jq.
set -euo pipefail
# A broken exchange must fail, not wait forever: the whole test is bounded.
if [[ -z ${NATIVE_LOOP_COUNTERS_BOUNDED:-} ]]; then
    NATIVE_LOOP_COUNTERS_BOUNDED=1 exec timeout --kill-after=10s 120s bash "${BASH_SOURCE[0]}" "$@"
fi
root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT
mkdir "$work/bin"
cat > "$work/client.rs" <<'RUST'
// Mirrors runtime/crates/indra-engine/examples/ocr/perf.rs: non-blocking FIFO
// opens, one command per exchange and exactly "ack\n\0" with nothing queued after.
use std::{
    fs::{File, OpenOptions},
    io::{ErrorKind, Read, Write},
    os::unix::fs::OpenOptionsExt,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

static NONBLOCK: i32 = 0o4000;
static STOP: AtomicBool = AtomicBool::new(false);
static LEAVE: AtomicBool = AtomicBool::new(false);

fn exchange(control: &mut File, ack: &mut File, command: &[u8]) {
    control.write_all(command).expect("control FIFO write");
    let due = Instant::now() + Duration::from_secs(5);
    let mut reply = [0_u8; 5];
    let mut received = 0;
    while received < reply.len() {
        match ack.read(&mut reply[received..]) {
            Ok(0) => panic!("acknowledgement FIFO closed"),
            Ok(count) => received += count,
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                assert!(Instant::now() < due, "acknowledgement deadline expired");
                thread::sleep(Duration::from_millis(1));
            }
            Err(error) => panic!("acknowledgement FIFO read: {error}"),
        }
    }
    assert_eq!(&reply, b"ack\n\0");
    let queued = ack.read(&mut [0_u8; 1]).map_err(|error| error.kind());
    assert_eq!(queued, Err(ErrorKind::WouldBlock), "queued acknowledgement bytes");
}

// Blocks a thousand times a second: voluntary switches inside the loop.
fn idle(flag: &'static AtomicBool) {
    while !flag.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(1));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args[1].as_str();
    if mode == "early-exit" {
        std::process::exit(43);
    }
    if mode == "idle" {
        thread::sleep(Duration::from_secs(60));
        return;
    }
    let mut ack = OpenOptions::new().read(true).custom_flags(NONBLOCK).open(&args[3]).expect("ack FIFO");
    let mut control = OpenOptions::new().write(true).custom_flags(NONBLOCK).open(&args[2]).expect("control FIFO");
    let resident = thread::spawn(|| idle(&STOP));
    let mut leaving = Some(thread::spawn(|| idle(&LEAVE)));
    if mode == "disable-first" {
        exchange(&mut control, &mut ack, b"disable\n");
    }
    exchange(&mut control, &mut ack, b"enable\n");
    let started = thread::spawn(|| idle(&STOP));
    if mode == "thread-exit" {
        LEAVE.store(true, Ordering::Relaxed);
        leaving.take().expect("leaving thread").join().expect("leaving thread");
    }
    thread::sleep(Duration::from_millis(100));
    exchange(&mut control, &mut ack, b"disable\n");
    STOP.store(true, Ordering::Relaxed);
    LEAVE.store(true, Ordering::Relaxed);
    for handle in [Some(resident), Some(started), leaving].into_iter().flatten() {
        handle.join().expect("thread");
    }
}
RUST
# Outside the checkout, so the repository toolchain file does not trigger an install.
(cd "$work" && rustc --edition=2024 -O client.rs -o bin/ocr-profiling)
# shellcheck source=scripts/profile-native.sh
source "$root/scripts/profile-native.sh"
exchange() {
    local mode=$1 control acknowledgement job other='' status=0 served=0
    phase_out=$work/$mode phase_group=$work/$mode-cgroup
    mkdir "$phase_out" "$phase_group"
    printf 'usage_usec 100\n' > "$phase_group/cpu.stat"
    mkfifo "$phase_out/loop-control.fifo" "$phase_out/loop-ack.fifo"
    exec {control}<> "$phase_out/loop-control.fifo" {acknowledgement}<> "$phase_out/loop-ack.fifo"
    "$work/bin/ocr-profiling" "$mode" "$phase_out/loop-control.fifo" "$phase_out/loop-ack.fifo" \
        {control}>&- {acknowledgement}>&- 2> "$phase_out/client.stderr" &
    job=$!
    # The fake cgroup holds only the client; the real one also holds this shell.
    printf '%s\n' "$job" > "$phase_group/cgroup.procs"
    if [[ $mode = duplicate ]]; then
        # A second OCR process in the cgroup: no snapshot may pick one of them.
        "$work/bin/ocr-profiling" idle {control}>&- {acknowledgement}>&- > /dev/null 2> "$phase_out/idle.stderr" &
        other=$!
        until [[ $(cat "/proc/$other/comm") = ocr-profiling ]]; do sleep 0.01; done
        printf '%s\n' "$other" >> "$phase_group/cgroup.procs"
    fi
    serve_loop_control "$job" "$control" "$acknowledgement" || served=$?
    wait "$job" || status=$?
    if [[ -n $other ]]; then kill "$other"; wait "$other" || true; fi
    exec {control}>&- {acknowledgement}>&-
    printf '%s %s\n' "$status" "$served"
}
summarize() {
    printf 'usage_usec 10\nuser_usec 6\nsystem_usec 4\nnr_periods 0\nnr_throttled 0\nthrottled_usec 0\n' > "$work/before"
    printf 'usage_usec 20\nuser_usec 12\nsystem_usec 8\nnr_periods 0\nnr_throttled 0\nthrottled_usec 0\n' > "$work/loop-before"
    printf 'usage_usec 90\nuser_usec 62\nsystem_usec 28\nnr_periods 0\nnr_throttled 0\nthrottled_usec 0\n' > "$work/loop-after"
    printf 'usage_usec 99\nuser_usec 66\nsystem_usec 33\nnr_periods 0\nnr_throttled 0\nthrottled_usec 0\n' > "$work/after"
    printf '1000\n' > "$work/peak"
    jq -nc 'range(32)|{elapsed_us:100,valid_columns:1,padded_columns:1,stage_us:[0,0,0,0,0,0,0]}' > "$work/requests"
    jq -n --argjson metadata '{}' --argjson iterations 32 --slurpfile requests "$work/requests" \
        --rawfile before "$work/before" --rawfile after "$work/after" --rawfile peak "$work/peak" \
        --rawfile loop_before "$work/loop-before" --rawfile loop_after "$work/loop-after" \
        --rawfile threads_before "$work/$1/loop-enable/threads.tsv" --rawfile threads_after "$work/$1/loop-disable/threads.tsv" \
        --from-file "$root/scripts/native-ablation-resources.jq"
}
[[ $(exchange complete) = '0 0' ]] || { cat "$work/complete/client.stderr" >&2; exit 1; }
[[ $(cat "$work/complete/loop-enable/pid") = "$(cat "$work/complete/loop-disable/pid")" ]]
cmp "$work/complete-cgroup/cpu.stat" "$work/complete/loop-enable/cpu.stat"
[[ $(wc -l < "$work/complete/loop-enable/threads.tsv") = 3 && $(wc -l < "$work/complete/loop-disable/threads.tsv") = 4 ]]
summarize complete > "$work/complete.json"
# The resident and started threads sleep about a hundred times inside the loop.
jq -e '.loop_threads==4 and .loop_voluntary_switches>=40 and .loop_cpu_usage_us==70 and .loop_cpu_system_us==20' \
    "$work/complete.json" > /dev/null || { cat "$work/complete.json" >&2; exit 1; }
# A client that dies before the exchange is noticed within about a second.
started=$SECONDS
[[ $(exchange early-exit) = '43 1' && $((SECONDS - started)) -le 5 ]]
[[ ! -e $work/early-exit/loop-enable/cpu.stat && ! -e $work/early-exit/loop-enable/threads.tsv ]]
# An out-of-order command is refused; the client then fails on its own deadline.
result=$(exchange disable-first)
[[ $result != '0 '* && $result = *' 1' && ! -e $work/disable-first/loop-enable/cpu.stat ]]
# Two OCR processes in the cgroup are refused before any counter is read.
result=$(exchange duplicate)
[[ $result != '0 '* && $result = *' 1' && ! -e $work/duplicate/loop-enable/cpu.stat ]]
# A thread that exits inside the loop would lose its switches: the summary refuses it.
[[ $(exchange thread-exit) = '0 0' ]]
if summarize thread-exit > "$work/thread-exit.json" 2> "$work/thread-exit.stderr"; then
    echo 'a thread that exited inside the loop was admitted' >&2; exit 1
fi
grep -Fq 'an OCR thread exited inside the measured loop' "$work/thread-exit.stderr"
printf '%s\n' 'Warmed-loop counter FIFO exchange contracts passed.'
