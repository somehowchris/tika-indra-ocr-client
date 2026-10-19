
      def counter($s;$k): $s|split("\n")|map(select(startswith($k+" ")))|
        if length!=1 then error("missing/duplicate cgroup counter") else .[0]|split(" ")[1]|tonumber end;
      ($requests|map(.elapsed_us)|sort) as $x |
      $metadata+{p50_us:$x[15],p95_us:$x[30],p99_us:$x[31],
        warmed_recognitions_per_second:(32000000/($x|add)),
        phase_cpu_usage_us:(counter($after;"usage_usec")-counter($before;"usage_usec")),
        phase_throttled_us:(counter($after;"throttled_usec")-counter($before;"throttled_usec")),
        phase_peak_memory_bytes:($peak|if test("^[0-9]+\\n\\z") then rtrimstr("\n")|tonumber else error("invalid cgroup peak counter") end),request_count:($requests|length),
        valid_columns:($requests|map(.valid_columns)|add),padded_columns:($requests|map(.padded_columns)|add)}
      | if .phase_cpu_usage_us<0 or .phase_throttled_us<0 or (.phase_peak_memory_bytes<=0 or .phase_peak_memory_bytes>3221225472) or .request_count!=32
        then error("invalid resource observation") else . end
