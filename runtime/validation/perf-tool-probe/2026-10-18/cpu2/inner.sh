set -euo pipefail
export LC_ALL=C
cd /evidence
start=$SECONDS
while [[ ! -f admitted ]]; do ((SECONDS-start<30)); sleep 0.02; done
[[ $(cat admitted) = admitted ]]
(cd / && sha256sum --check --strict /profile-tools/build-identities.sha256) > tools-verification.txt
/usr/bin/perf version --build-options > perf-version.txt
mkfifo control.fifo ack.fifo
cat /proc/$$/limits > process.limits; cat /proc/$$/status > process.status; cat /proc/sys/kernel/perf_event_mlock_kb > perf_event_mlock_kb; /usr/bin/perf record -vv --no-buildid-cache --delay=-1 --event cpu-clock:u --freq 99 --strict-freq \
 --call-graph dwarf,16384 --mmap-pages=32M --max-size=256M --output perf.data \
 --control=fifo:control.fifo,ack.fifo -- /bin/bash -euo pipefail -c '
 exec 3<>control.fifo 4<>ack.fifo
 printf "enable\n" >&3
 read -r -t5 acknowledgement <&4
 [[ $acknowledgement = ack ]]
 status=0
 timeout --signal=TERM --kill-after=1s 1s /bin/bash -c "while :; do :; done" || status=$?
 [[ $status = 124 ]]
 printf "disable\n" >&3
 read -r -t5 acknowledgement <&4
 [[ $acknowledgement = ack ]]
 ' > record.stdout 2> record.stderr
/usr/bin/perf report --stdio --header --show-nr-samples --no-children --percent-limit 0 --sort dso,symbol --input perf.data > perf-report.txt 2> report.stderr
/usr/bin/perf script --header --show-lost-events -F comm,pid,tid,time,event,ip,sym,dso --input perf.data > perf-script.txt 2> script.stderr
[[ -s perf.data && -s perf-report.txt && -s perf-script.txt ]]
grep -q 'cpu-clock:u' perf-script.txt
cat /sys/fs/cgroup/memory.peak > memory.peak; cat /sys/fs/cgroup/memory.events > memory.events; cat /sys/fs/cgroup/cpu.max > cpu.max
