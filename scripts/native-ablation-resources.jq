      def counter($s;$k): $s|split("\n")|map(select(startswith($k+" ")))|
        if length!=1 then error("missing/duplicate cgroup counter") else .[0]|split(" ")[1]|tonumber end;
      def delta($k): (counter($after;$k)-counter($before;$k));
      def loop($k): (counter($loop_after;$k)-counter($loop_before;$k));
      # Nearest-rank quantile of n sorted values, as in the 2026-10-18 analysis.
      def rank($q): .[((length*$q|ceil)-1)];
      # tid, name, voluntary and involuntary switches of every OCR thread.
      def threads($s): $s|split("\n")|map(select(length>0)|split("\t")|
        if length!=4 or (.[0]|test("^[1-9][0-9]*$")|not) or (.[2]|test("^[0-9]+$")|not) or (.[3]|test("^[0-9]+$")|not)
        then error("invalid thread counter record") else {key:.[0],value:{voluntary:(.[2]|tonumber),involuntary:(.[3]|tonumber)}} end)|
        if length==0 or (map(.key)|unique|length)!=length then error("missing/duplicate OCR thread") else from_entries end;
      (threads($threads_before)) as $tb | (threads($threads_after)) as $ta |
      # A thread started inside the loop counts in full; one that exited would lose its switches.
      (if ($tb|keys)-($ta|keys)!=[] then error("an OCR thread exited inside the measured loop") else . end) |
      ([$ta|to_entries[]|.key as $tid|.value as $now|($tb[$tid]//{voluntary:0,involuntary:0}) as $was|
        {voluntary:($now.voluntary-$was.voluntary),involuntary:($now.involuntary-$was.involuntary)}]|
        if any(.[]; .voluntary<0 or .involuntary<0) then error("decreasing thread switch counter") else . end) as $switches |
      ($requests|map(.elapsed_us)|sort) as $x | ($requests|length) as $n |
      $metadata+{p50_us:($x|rank(0.50)),p95_us:($x|rank(0.95)),p99_us:($x|rank(0.99)),
        warmed_recognitions_per_second:($n*1000000/($x|add)),
        phase_cpu_usage_us:delta("usage_usec"),
        phase_throttled_us:delta("throttled_usec"),
        phase_peak_memory_bytes:($peak|if test("^[0-9]+\\n\\z") then rtrimstr("\n")|tonumber else error("invalid cgroup peak counter") end),request_count:$n,
        valid_columns:($requests|map(.valid_columns)|add),padded_columns:($requests|map(.padded_columns)|add),
        stage_p50_us:[range(0;7) as $stage|$requests|map(.stage_us[$stage])|sort|rank(0.50)],
        loop_cpu_usage_us:loop("usage_usec"),loop_cpu_user_us:loop("user_usec"),loop_cpu_system_us:loop("system_usec"),
        loop_periods:loop("nr_periods"),loop_throttled_periods:loop("nr_throttled"),loop_throttled_us:loop("throttled_usec"),
        loop_voluntary_switches:($switches|map(.voluntary)|add),loop_involuntary_switches:($switches|map(.involuntary)|add),
        loop_threads:($ta|length)}
      | . + {loop_cpu_us_per_request:(.loop_cpu_usage_us/$n),loop_system_us_per_request:(.loop_cpu_system_us/$n),
             loop_throttled_us_per_request:(.loop_throttled_us/$n),
             loop_switches_per_request:((.loop_voluntary_switches+.loop_involuntary_switches)/$n)}
      | if .phase_cpu_usage_us<0 or .phase_throttled_us<0 or (.phase_peak_memory_bytes<=0 or .phase_peak_memory_bytes>3221225472) or .request_count!=$iterations
          or .loop_cpu_usage_us<=0 or .loop_cpu_user_us<0 or .loop_cpu_system_us<0 or .loop_periods<0 or .loop_throttled_periods<0 or .loop_throttled_us<0
          or ($requests|any(.stage_us|length!=7))
          or counter($loop_before;"usage_usec")<counter($before;"usage_usec") or counter($after;"usage_usec")<counter($loop_after;"usage_usec")
        then error("invalid resource observation") else . end
