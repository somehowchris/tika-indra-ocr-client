# Input: analysis.json of the thread-policy study. Output: the Markdown decision table.
def r: if . == null then "–" else (. * 1000 | round / 1000 | tostring) end;
def span: if length == 0 then "–" else "\(min|r) / \(sort|.[length/2|floor]|r) / \(max|r)" end;
def us: . / 1000 | . * 10 | round / 10 | tostring;
.decisions as $d |
"| Placement | Candidate | Fixture | Median ratio min / mid / max | Largest p95 ratio | Loop CPU ratio min / mid / max | Switch ratio mid | Detection-stage ratio | Class | Promoted |",
"| --- | --- | --- | --- | ---: | --- | ---: | ---: | --- | --- |",
($d.placements[] as $place | $place.candidates[] as $c | $c.fixtures[] |
  "| \($place.placement) (\($place.millicpus / 1000) CPU, \($place.cpuset)) | \($c.comparison) | \(.fixture) | \(.median_ratios|span) | \(.p95_ratios|max|r) | \(.cpu_ratios|span) | \(.switch_ratios|map(select(. != null))|if length == 0 then "–" else (sort|.[length/2|floor]|r) end) | \(.stage_ratio_medians[2]|r) | \($c.class) | \(if $c.promoted then "yes" else "no" end) |"),
"",
($d.placements[] | "- \(.placement): selected \(.selected)\(if .class then " (\(.class))" else "" end)."),
"- Shipped two-CPU profile: \($d.shipped_two_cpu_profile | if .status == "keep A" then "keep A" else "\(.setting) is a candidate for whole-Tika confirmation before any change to \(.files|join(" and "))" end); \($d.shipped_two_cpu_profile.reason).",
($d.per_session_detector[] | "- Per-session detector threads at \(.placement): \(if .worth_implementing then "worth implementing" else "rejected" end)."),
"",
"| Placement | Fixture | A processes | A median p50 ms | A median p95 ms | A loop CPU ms/request | A throttled ms/request | A switches/request |",
"| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |",
($d.baseline_by_placement[] |
  "| \(.placement) | \(.fixture) | \(.processes) | \(.median_p50_us|us) | \(.median_p95_us|us) | \(.median_loop_cpu_us_per_request|us) | \(.median_loop_throttled_us_per_request|us) | \(.median_loop_switches_per_request|round) |")
