#!/usr/bin/env bash
# Resolve the systemd Docker scope containing kind's host PID. systemd inside
# the node can move PID 1 into init.scope; limits remain on its Docker ancestor.
kind_cgroup_owner() {
    local id=$1 record=$2 line relative component current='' owner='' depth=0 matches=0
    [[ $id =~ ^[0-9a-f]{64}$ && -f $record && ! -L $record && $(wc -c < "$record") -le 4096 ]] || return 1
    IFS= read -r line < "$record" || return 1
    [[ $(wc -l < "$record") = 1 && $line == 0::/* ]] || return 1
    LC_ALL=C grep -aEx '^0::/[a-zA-Z0-9_.:@/-]+$' "$record" | cmp -s -- "$record" - || return 1
    relative=${line#0::}
    [[ $relative != */ && $relative != *//* && $relative =~ ^/[a-zA-Z0-9_.:@/-]+$ ]] || return 1
    local -a components
    IFS=/ read -r -a components <<< "${relative#/}"
    for component in "${components[@]}"; do
        depth=$((depth+1)); (( depth <= 64 )) || return 1
        [[ $component != . && $component != .. ]] || return 1
        current+=/$component
        if [[ $component == docker-"$id".scope ]]; then
            owner=$current; matches=$((matches+1))
        elif [[ $component =~ ^docker-[0-9a-f]{64}\.scope$ ]]; then
            return 1
        fi
    done
    (( matches == 1 )) || return 1
    printf '%s\n' "$owner"
}

kind_cgroup_quota() {
    local record=$1 cpus=$2 quota period extra
    [[ $cpus =~ ^[12]$ && -f $record && ! -L $record && $(wc -c < "$record") -le 128 && $(wc -l < "$record") = 1 ]] || return 1
    LC_ALL=C grep -aEx '^[0-9]+ [1-9][0-9]*$' "$record" | cmp -s -- "$record" - || return 1
    read -r quota period extra < "$record" || return 1
    [[ $quota =~ ^[0-9]+$ && $period =~ ^[1-9][0-9]*$ && -z $extra && ${#quota} -le 12 && ${#period} -le 12 ]] || return 1
    (( 10#$quota == 10#$period * cpus ))
}
