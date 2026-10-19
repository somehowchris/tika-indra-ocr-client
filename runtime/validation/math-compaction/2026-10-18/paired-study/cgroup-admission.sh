#!/usr/bin/env bash
# Read-only host cgroup admission. Caller must first bind the live host PID and
# Docker leaf to its owned immutable container; this helper changes no cgroups.
attest_host_ancestors() {
    local current=$1 cpus=$2 destination=$3 quota period extra memory depth=0
    [[ $current = /sys/fs/cgroup/* && -d $current && $cpus =~ ^[12]$ ]] || return 1
    : > "$destination" || return
    while :; do
        depth=$((depth+1)); ((depth<=64)) || return 1
        printf '%s\n' "$current" >> "$destination" || return
        if [[ -f $current/cpu.max ]]; then
            read -r quota period extra < "$current/cpu.max" || return
            printf 'cpu.max %s %s\n' "$quota" "$period" >> "$destination" || return
            [[ $period =~ ^[1-9][0-9]*$ && -z $extra ]] || return 1
            if [[ $quota != max ]]; then
                [[ $quota =~ ^[0-9]+$ ]] && ((quota>=cpus*period)) || return 1
            fi
        fi
        if [[ -f $current/memory.max ]]; then
            IFS= read -r memory < "$current/memory.max" || return
            printf 'memory.max %s\n' "$memory" >> "$destination" || return
            if [[ $memory != max ]]; then
                [[ $memory =~ ^[0-9]+$ ]] && ((memory>=3221225472)) || return 1
            fi
        fi
        [[ $current = /sys/fs/cgroup ]] && break
        current=$(dirname "$current") || return
        [[ $current = /sys/fs/cgroup || $current = /sys/fs/cgroup/* ]] || return 1
    done
}
