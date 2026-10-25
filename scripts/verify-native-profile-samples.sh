#!/usr/bin/env bash
# Verify every sampled process against the exact OCR participant, not its name.
set -euo pipefail
set -o noclobber
export LC_ALL=C
fail() { printf 'native profile samples rejected: %s\n' "$*" >&2; exit 1; }
[[ $# = 7 ]] || fail 'usage: verify-native-profile-samples.sh REPORT SCRIPT STATS FOLDED DEMANGLED_REPORT DEMANGLED_FOLDED NEW_JSON'
report=$1
trace=$2
stats=$3
folded_file=$4
demangled_report=$5
demangled_folded=$6
output=$7
for file in "$report" "$trace" "$stats" "$folded_file" "$demangled_report" "$demangled_folded"; do
    [[ -f $file && -r $file ]] || fail 'an input is not a readable regular file'
    size=$(stat -c '%s' -- "$file")
    if [[ ! $size =~ ^[0-9]+$ ]] || ((size == 0 || size > 1073741824)); then
        fail 'input exceeds the finite byte bound'
    fi
done
[[ ! -e $output && ! -L $output ]] || fail 'refusing to replace an existing observation'
pid=$(jq -se '
    if length==64 and map(.iteration)==[range(0;64)] and
       all(.[]; .process_id | type=="number" and isfinite and floor==. and .>0 and .<=4294967295) and
       ([.[].process_id]|unique|length)==1
    then .[0].process_id else error("missing, changed or invalid participant PID") end' "$stats") \
    || fail 'the complete request journal does not identify one participant'
lost=$(awk '
    /^# Total Lost Samples:[[:space:]]+[0-9]+$/ {n++; value=$5}
    END {if(n!=1) exit 1; print value}' "$report") || fail 'missing or ambiguous lost-sample counter'
# Keep the machine observation exact in jq/IEEE-754 and avoid shell integer
# overflow. Every nonzero counter already fails the zero-loss admission rule.
[[ $lost =~ ^(0|[1-9][0-9]{0,15})$ ]] || fail 'unrepresentable lost-sample counter'
((lost <= 9007199254740991)) || fail 'unrepresentable lost-sample counter'
read -r samples unknown extra < <(awk -v expected="$pid" '
    function reject(reason) {print reason > "/dev/stderr"; bad=1; exit 1}
    /^[[:space:]]*(#|$)/ {next}
    /cpu-clock:u:/ {
        if (!match($0, /^[[:space:]]*.+[[:space:]][0-9]+\/[0-9]+[[:space:]]+[0-9]+\.[0-9]+:[[:space:]]+cpu-clock:u:/))
            reject("invalid sampled process/thread/time header")
        fields=substr($0,RSTART,RLENGTH)
        sub(/[[:space:]]+[0-9]+\.[0-9]+:[[:space:]]+cpu-clock:u:$/, "", fields)
        sub(/^.*[[:space:]]/, "", fields)
        split(fields, ids, "/")
        if (ids[1]+0 != expected+0 || ids[2]+0 <= 0 || ids[2]+0 > 4294967295)
            reject("sample belongs to a different process or an invalid thread")
        unknown+=pending_unknown
        pending_unknown=($0 ~ /\[unknown\]/)
        active=1
        samples++
        next
    }
    /[0-9]+\.[0-9]+:[[:space:]]+[^[:space:]]+:/ {
        reject("unexpected event in the sampled record stream")
    }
    active && /^[[:space:]]+[[:xdigit:]]+[[:space:]].*[[:space:]]\([^)]*\)[[:space:]]*$/ {
        if ($0 ~ /\[unknown\]/) pending_unknown=1
        next
    }
    {reject("unrecognized record in the sampled stream")}
    END {
        if (bad) exit 1
        if (samples==0) {print "no recognized sample headers" > "/dev/stderr"; exit 1}
        printf "%.0f %.0f\n", samples, unknown+pending_unknown
    }' "$trace") || fail 'sample header admission failed'
[[ $samples =~ ^[0-9]+$ && $unknown =~ ^[0-9]+$ && -z $extra ]] || fail 'invalid sample counters'
folded=$(awk '
    NF<2 || $NF !~ /^[0-9]+$/ || $NF+0<=0 {bad=1; exit 1}
    {sum+=$NF}
    END {if(bad || sum<=0) exit 1; printf "%.0f\n",sum}' "$folded_file") \
    || fail 'invalid collapsed sample weights'
# Demangling may rewrite only mangled Rust symbols: every folded stack keeps
# its position, frame count and weight, every report line its position, and a
# frame or report line without a mangled symbol stays byte-identical. Lines
# that still carry a mangled Rust symbol after demangling are counted.
read -r demangled conserved residual extra < <(awk '
    function mangled(text) {return text ~ /_R[0-9]*[CMXYNIB][0-9A-Za-z_]/ || text ~ /_ZN[0-9]+[A-Za-z_]/}
    function rust(text) {return text ~ /_R[0-9]*[CMXYNIB][0-9A-Za-z_]/ || text ~ /_ZN[0-9]+[0-9A-Za-z_$.]*17h[0-9a-f]+E/}
    FNR == 1 {part++}
    part == 1 {folded[FNR] = $0; folded_lines = FNR; next}
    part == 2 {
        if (!match($0, / [1-9][0-9]*$/)) {bad = 1; exit 1}
        stack = substr($0, 1, RSTART - 1); count = substr($0, RSTART + 1)
        sum += count; demangled_lines = FNR
        if (rust(stack)) residual++
        if (FNR > folded_lines || !match(folded[FNR], / [0-9]+$/)) {differs = 1; next}
        if (substr(folded[FNR], RSTART + 1) != count) differs = 1
        frames = split(substr(folded[FNR], 1, RSTART - 1), before, ";")
        if (split(stack, after, ";") != frames) differs = 1
        for (i = 1; i <= frames; i++) if (!mangled(before[i]) && before[i] != after[i]) differs = 1
        next
    }
    part == 3 {report[FNR] = $0; report_lines = FNR; next}
    {
        if (FNR > report_lines || (!mangled(report[FNR]) && report[FNR] != $0)) differs = 1
        if (rust($0)) residual++
        demangled_report_lines = FNR
    }
    END {
        if (bad || part != 4) exit 1
        if (demangled_lines != folded_lines || demangled_report_lines != report_lines) differs = 1
        printf "%.0f %s %.0f\n", sum, (differs ? "false" : "true"), residual
    }' "$folded_file" "$demangled_folded" "$report" "$demangled_report") || fail 'invalid demangled folded stacks or report'
[[ $demangled =~ ^[0-9]+$ && $conserved =~ ^(true|false)$ && $residual =~ ^[0-9]+$ && -z $extra ]] \
    || fail 'invalid demangling counters'
jq -n --argjson process_id "$pid" --argjson samples "$samples" --argjson folded "$folded" \
    --argjson lost "$lost" --argjson unknown "$unknown" --argjson demangled "$demangled" \
    --argjson conserved "$conserved" --argjson residual "$residual" '
    {process_id:$process_id,event:"cpu-clock:u",frequency_hz:99,samples:$samples,
     folded_samples:$folded,lost_samples:$lost,samples_with_unknown_frames:$unknown,
     demangled_folded_samples:$demangled,demangling_conserved:$conserved,
     lines_with_mangled_rust_symbols:$residual,
     target:"one executed OCR process and its inherited threads",
     all_sampled_process_ids_match:true,visual_review:"pending",
     accepted:($samples>=100 and $folded==$samples and $lost==0 and
       $conserved and $demangled==$folded and $residual==0)}' > "$output"
jq -e '.accepted==true' "$output" > /dev/null || fail 'sampling quality gate failed'
