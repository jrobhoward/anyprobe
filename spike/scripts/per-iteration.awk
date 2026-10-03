# Checks per-iteration probe counts for the spike's attach scripts.
#
# Input: map dumps in bpftrace's format, which attach-linux-stap.sh prints too:
#   @e[ID, LABEL]: N   work__entry fired N times with this label in iteration ID
#   @r[ID]: N          work__return fired N times in iteration ID
# An iteration is complete when all 4 labels fired once and work__return fired
# 4 times. Output: one line per iteration that broke the rules (a count too
# high, or a partial iteration inside the complete run), then
# "full=<number of complete iterations>".
/^@e\[[0-9]+, .*\]: [0-9]+$/ {
  line = $0; sub(/^@e\[/, "", line)
  id = line; sub(/,.*/, "", id)
  label = line; sub(/^[0-9]+, /, "", label); sub(/\]: [0-9]+$/, "", label)
  n = line; sub(/.*\]: /, "", n)
  if (n != 1) print "iteration " id ": label " label " fired " n " times"
  labels[id]++; ids[id] = 1
}
/^@r\[[0-9]+\]: [0-9]+$/ {
  id = $0; sub(/^@r\[/, "", id); sub(/\].*/, "", id)
  n = $0; sub(/.*\]: /, "", n)
  if (n + 0 > 4) print "iteration " id ": work__return fired " n " times"
  ret[id] = n + 0; ids[id] = 1
}
END {
  lo = -1; hi = -1; full = 0
  for (i in ids) if (labels[i] == 4 && ret[i] == 4) {
    full++
    if (lo < 0 || i + 0 < lo) lo = i + 0
    if (hi < 0 || i + 0 > hi) hi = i + 0
  }
  if (full > 0 && hi - lo + 1 != full)
    print "complete iterations " lo ".." hi " have gaps (" full " complete)"
  for (i in ids) if (!(labels[i] == 4 && ret[i] == 4) && i + 0 > lo && i + 0 < hi)
    print "iteration " i " is partial inside the complete run"
  print "full=" full
}
