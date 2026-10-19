# Input: complete, admitted process-level summaries. Requests are not independent replicates.
if length!=128 then error("incomplete ablation schedule") else . end |
group_by([.cpus,.fixture,.comparison,.block]) |
map(if length!=2 or (map(.arm)|sort)!=["A","B"] then error("incomplete pair") else . end |
  (map(select(.arm=="A"))[0]) as $a | (map(select(.arm=="B"))[0]) as $b |
  {cpus:$a.cpus,fixture:$a.fixture,comparison:$a.comparison,block:$a.block,order:$a.order,
   baseline:$a,candidate:$b,median_ratio:($b.p50_us/$a.p50_us),
   p95_ratio:($b.p95_us/$a.p95_us),peak_memory_ratio:($b.phase_peak_memory_bytes/$a.phase_peak_memory_bytes)}) |
. as $pairs | {scope:"Four descriptive process pairs per CPU/fixture/comparison; no request-level independent inference",
  pairs:$pairs,decisions:($pairs|group_by(.comparison)|map({comparison:.[0].comparison,
    decision:(if all(.[]; .median_ratio<=0.95 and .p95_ratio<=1.05 and .peak_memory_ratio<=1.10)
      then "candidate meets conservative synthetic-fixture policy; confirm on representative workload"
      else "retain baseline; candidate fails conservative synthetic-fixture policy" end)}))}
