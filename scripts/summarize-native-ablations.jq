# Input: complete, admitted process-level summaries; $protocol: the frozen protocol.
# Requests are not independent replicates.
$protocol[0] as $p |
(if $p.placements then "placement" else "cpus" end) as $axis |
(($p.placements // $p.cpus | length) * ($p.fixtures | length) * ($p.comparisons | length) *
  ($p.paired_block_order | length) * 2) as $processes |
if length!=$processes then error("incomplete ablation schedule") else . end |
group_by([.[$axis],.fixture,.comparison,.block]) |
map(if length!=2 or (map(.arm)|sort)!=["A","B"] then error("incomplete pair") else . end |
  (map(select(.arm=="A"))[0]) as $a | (map(select(.arm=="B"))[0]) as $b |
  {($axis):$a[$axis],fixture:$a.fixture,comparison:$a.comparison,block:$a.block,order:$a.order,
   baseline:$a,candidate:$b,median_ratio:($b.p50_us/$a.p50_us),
   p95_ratio:($b.p95_us/$a.p95_us),peak_memory_ratio:($b.phase_peak_memory_bytes/$a.phase_peak_memory_bytes)} +
  if $axis=="placement" then
    {cpu_ratio:($b.loop_cpu_us_per_request/$a.loop_cpu_us_per_request),
     system_cpu_ratio:(if $a.loop_cpu_system_us>0 then $b.loop_cpu_system_us/$a.loop_cpu_system_us else null end),
     switch_ratio:(if $a.loop_switches_per_request>0 then $b.loop_switches_per_request/$a.loop_switches_per_request else null end),
     stage_ratios:[range(0;7) as $s|if $a.stage_p50_us[$s]>0 then $b.stage_p50_us[$s]/$a.stage_p50_us[$s] else null end]}
  else {} end) |
. as $pairs |
def median: sort | if length%2==1 then .[length/2|floor] else (.[length/2-1]+.[length/2])/2 end;
# Schema 1: one conservative rule across every CPU budget and fixture.
def every_cell: $pairs|group_by(.comparison)|map({comparison:.[0].comparison,
  decision:(if all(.[]; .median_ratio<=0.95 and .p95_ratio<=1.05 and .peak_memory_ratio<=1.10)
    then "candidate meets conservative synthetic-fixture policy; confirm on representative workload"
    else "retain baseline; candidate fails conservative synthetic-fixture policy" end)});
# Schema 2: each placement is decided on its own pairs; see decision_policy.
def per_placement: $p.decision_policy as $d |
  [$p.placements[] as $place | ($p.comparisons|to_entries[]) as $c |
    [$pairs[]|select(.placement==$place.id and .comparison==$c.value.id)] as $own |
    ($own|map(.median_ratio)|max) as $worst | ($own|map(.cpu_ratio)|max) as $costliest |
    (if all($own[]; .median_ratio<=$d.faster_max_median_ratio) then "faster"
     elif all($own[]; .median_ratio<=$d.cheaper_max_median_ratio and .cpu_ratio<=$d.cheaper_max_cpu_ratio) then "cheaper"
     else "neither" end) as $class |
    {placement:$place.id,comparison:$c.value.id,order:$c.key,class:$class,
     guards:all($own[]; .p95_ratio<=$d.max_p95_ratio and .peak_memory_ratio<=$d.max_peak_memory_ratio),
     largest_median_ratio:$worst,largest_cpu_ratio:$costliest,
     fixtures:($own|group_by(.fixture)|map({fixture:.[0].fixture,
       median_ratios:map(.median_ratio),p95_ratios:map(.p95_ratio),cpu_ratios:map(.cpu_ratio),
       switch_ratios:map(.switch_ratio),peak_memory_ratios:map(.peak_memory_ratio),
       stage_ratio_medians:[range(0;7) as $s|map(.stage_ratios[$s])|if any(.[]; .==null) then null else median end]}))}
    | . + {promoted:(.class!="neither" and .guards)}] as $candidates |
  [$p.placements[] as $place |
    # A faster candidate always has the lower largest median ratio, so it ranks first.
    ([$candidates[]|select(.placement==$place.id and .promoted)]|
      sort_by([.largest_median_ratio,.largest_cpu_ratio,.order])|first) as $chosen |
    {placement:$place.id,millicpus:$place.millicpus,cpuset:$place.cpuset,
     selected:($chosen.comparison//"A"),class:($chosen.class//null),
     candidates:[$candidates[]|select(.placement==$place.id)|del(.order)]}] as $placements |
  ($placements|map({key:.placement,value:.})|from_entries) as $by |
  def pairs_of($place;$comparison): [$pairs[]|select(.placement==$place and .comparison==$comparison)];
  {placements:$placements,
   # Engine pairs alone change no shipped file; see whole_tika_confirmation.
   shipped_two_cpu_profile:($by["shipped-ocr-quota"].selected as $x |
     if $x=="A" then {status:"keep A",setting:"A",reason:"no candidate is promoted in shipped-ocr-quota"}
     elif all(pairs_of("distinct-cores";$x)[]; .median_ratio<=1.05)
     then {status:"candidate for whole-Tika confirmation",setting:$x,files:["compose.2cpu.yaml","deploy/overlays/2cpu/resources.yaml"],
       reason:"promoted in shipped-ocr-quota and no distinct-cores pair median ratio above 1.05; the files change only after the paired whole-Tika confirmation at the shipped OCR 1.5 / Tika 0.5 partition passes"}
     else {status:"keep A",setting:"A",reason:"\($x) is promoted in shipped-ocr-quota but a distinct-cores pair median ratio exceeds 1.05; the shipped profile pins no CPUs"} end),
   per_session_detector:[$p.placements[].id as $place |
     [$pairs[]|select(.placement==$place and .fixture=="math")] as $math |
     {placement:$place,
      worth_implementing:(all($math[]|select(.comparison=="C-t1-cv1"); .stage_ratios[2]>=1.05 and
          .stage_ratios[1]<=1.00 and .stage_ratios[3]<=1.00 and .stage_ratios[4]<=1.00) and
        all($math[]|select(.comparison=="B-t2-cv1"); .stage_ratios[2]<=1.00))}],
   baseline_by_placement:[$p.placements[].id as $place | $p.fixtures[] as $fixture |
     [$pairs[]|select(.placement==$place and .fixture==$fixture)|.baseline] as $a |
     {placement:$place,fixture:$fixture,processes:($a|length),
      median_p50_us:($a|map(.p50_us)|median),median_p95_us:($a|map(.p95_us)|median),
      median_loop_cpu_us_per_request:($a|map(.loop_cpu_us_per_request)|median),
      median_loop_throttled_us_per_request:($a|map(.loop_throttled_us_per_request)|median),
      median_loop_switches_per_request:($a|map(.loop_switches_per_request)|median)}]};
{scope:(if $axis=="placement"
   then "Four descriptive process pairs per placement/fixture/comparison; no request-level independent inference"
   else "Four descriptive process pairs per CPU/fixture/comparison; no request-level independent inference" end),
 pairs:$pairs,
 decisions:(if $axis=="placement" then per_placement else every_cell end)}
