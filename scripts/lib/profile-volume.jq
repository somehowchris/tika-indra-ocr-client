# Only a unique controller-owned ordinary local volume is admitted.
length == 1 and
(.[0] | .Name == $name and .Driver == "local" and .Scope == "local" and
 .Options == null and (.Labels | type == "object") and
 (.Labels | keys) == [$label] and .Labels[$label] == $owner)
