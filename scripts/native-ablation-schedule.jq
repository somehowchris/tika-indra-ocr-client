# One baseline/candidate process pair; AB,BA,BA,AB balances order within each cell.
# Schema 1 varies the total CPU count; schema 2 names placements (quota and cpuset role).
. as $p | [
  ($p.placements // ($p.cpus | map({cpus:.})))[] as $place |
  $p.fixtures[] as $fixture |
  $p.comparisons[] as $comparison |
  ($p.paired_block_order | to_entries[]) as $block |
  ($block.value | split(""))[] as $arm |
  ($place | if has("id") then {placement:.id, millicpus, cpuset} else . end) +
  {fixture:$fixture, comparison:$comparison.id, block:$block.key,
   order:$block.value, arm:$arm,
   settings:(if $arm=="A" then $p.baseline else ($comparison|del(.id)) end)}
] | to_entries | map(.value + {id:("cell-"+((.key+1)|tostring))})
