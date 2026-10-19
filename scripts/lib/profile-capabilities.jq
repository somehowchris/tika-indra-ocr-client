# Docker may return the canonical CAP_ spelling for the requested capability.
# Normalize that prefix only; duplicates and every additional capability fail.
def profile_capabilities($phase):
  (.HostConfig.CapAdd | if . == null then [] else . end) as $actual |
  ($actual | type == "array") and
  ($actual | all(.[]; type == "string")) and
  ($actual | map(sub("^CAP_"; ""))) ==
    (if ($phase == "tiff" or $phase == "math") then ["PERFMON"] else [] end);
