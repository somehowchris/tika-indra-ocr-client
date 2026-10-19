set -euo pipefail
export LC_ALL=C CRITERION_HOME=/evidence/criterion
start=$SECONDS
while [[ ! -f /evidence/admitted ]]; do ((SECONDS-start<30)); sleep 0.02; done
[[ ! -L /evidence/admitted && $(stat -c%s /evidence/admitted) = 9 && $(cat /evidence/admitted) = admitted ]]
snapshot() {
 mkdir "/evidence/$1" || return
 for name in cpu.max cpu.stat cpuset.cpus.effective memory.max memory.swap.max memory.peak memory.events pids.max; do cat "/sys/fs/cgroup/$name" > "/evidence/$1/$name" || return; done
 cat /proc/self/status > "/evidence/$1/process.status" || return
 cat /proc/self/limits > "/evidence/$1/process.limits" || return
}
snapshot before
status=0
/usr/bin/timeout --signal=TERM --kill-after=5s 180s /bench --bench --noplot --warm-up-time 1 --measurement-time 3 --sample-size 100 > /evidence/criterion.stdout 2> /evidence/criterion.stderr || status=$?
printf '%s\n' "$status" > /evidence/criterion.exit
snapshot after
exit "$status"
