# One baseline/candidate process pair; AB,BA,BA,AB balances order within each cell.
. as $p | [
  $p.cpus[] as $cpus | $p.fixtures[] as $fixture |
  $p.comparisons[] as $comparison |
  ($p.paired_block_order | to_entries[]) as $block |
  ($block.value | split(""))[] as $arm |
  {cpus:$cpus, fixture:$fixture, comparison:$comparison.id, block:$block.key,
   order:$block.value, arm:$arm,
   settings:(if $arm=="A" then $p.baseline else ($comparison|del(.id)) end)}
] | to_entries | map(.value + {id:("cell-"+((.key+1)|tostring))})
