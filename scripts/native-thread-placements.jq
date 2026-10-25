# Input: sysfs topology of the allowed CPUs, [{cpu,core_id,package,siblings}] with
# siblings as the raw thread_siblings_list. Output: the cpuset of every role.
def expand:
  rtrimstr("\n") | split(",") | map(
    if test("^[0-9]+-[0-9]+$") then (split("-") | map(tonumber)) as [$a, $b] |
      if $a <= $b then range($a; $b + 1) else error("descending CPU range") end
    elif test("^[0-9]+$") then tonumber
    else error("invalid thread_siblings_list") end);
if length == 0 or (map(.cpu) != (map(.cpu) | unique)) then error("allowed CPUs must be unique and ascending") else . end |
map(. + {siblings:(.siblings | expand)}) |
(map(.cpu)) as $allowed |
(map({key:(.cpu|tostring), value:.}) | from_entries) as $by |
if any(.[]; .cpu as $cpu | .siblings | any(. == $cpu) | not) then error("a CPU is missing from its own thread_siblings_list") else . end |
([.[] | .cpu as $cpu | [.siblings[] | select(. != $cpu and IN($allowed[]))] | select(length > 0) | [$cpu, min]] | first
 // error("no two allowed CPUs are SMT siblings of one core")) as [$first, $sibling] |
$by[$first|tostring] as $one | $by[$sibling|tostring] as $two |
if ($two.siblings | any(. == $first) | not) or $two.core_id != $one.core_id or $two.package != $one.package
then error("CPUs \($first) and \($sibling) disagree about their shared core") else . end |
([.[] | select(.package == $one.package and (.cpu | IN($one.siblings[]) | not)) | .cpu] | first
 // error("no allowed CPU is on another physical core of CPU \($first)'s package")) as $distinct |
if $by[$distinct|tostring].core_id == $one.core_id
then error("CPU \($distinct) is outside CPU \($first)'s siblings but reports the same core") else . end |
# Cpusets are ascending, as the kernel reports an effective cpuset.
{"cpu-1":"\($first)", "cpu-2":"\($first),\($sibling)",
 "smt-siblings":"\($first),\($sibling)", "distinct-cores":([$first, $distinct] | sort | map(tostring) | join(","))}
