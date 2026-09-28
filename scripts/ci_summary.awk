# Compact failure summary of one CI stage log (scripts/ci.sh): each failed test with its location, message and a command that runs it
# alone; compiler errors with their location; formatting diffs. Variables: flags (extra cargo flags for repro lines), max (line cap).
function target(line,    m) {
  if (line ~ /Doc-tests /) return "--doc"
  if (match(line, /tests[\\\/][A-Za-z0-9_]+\.rs/)) return "--test " substr(line, RSTART + 6, RLENGTH - 9)
  if (line ~ /src[\\\/]lib\.rs/) return "--lib"
  if (line ~ /src[\\\/]main\.rs/) return "--bin red_engine2"
  if (match(line, /src[\\\/]bin[\\\/][A-Za-z0-9_]+/)) return "--bin " substr(line, RSTART + 8, RLENGTH - 8)
  return tgt
}
function out(s) { if (n < max) print s; n++ }
BEGIN { tgt = ""; n = 0; if (max == "") max = 40 }
/^ *(Running |Doc-tests )/ { tgt = target($0); next }
/^test .* \.\.\. FAILED$/ { name = $2; out("  FAILED " tgt " " name); out("    repro: cargo test" flags " " tgt " -- " name " --exact"); next }
/panicked at/ { loc = $0; sub(/.*panicked at /, "", loc); sub(/:$/, "", loc); getline msg; gsub(/^ +/, "", msg); out("    at " loc ": " substr(msg, 1, 160)); next }
/^error(\[E[0-9]+\])?: / && !/test failed, to rerun/ && !/targets? failed/ { e = $0; getline loc; if (loc ~ /-->/) { gsub(/^ +/, "", loc); e = e "  " loc }; out("  " e); next }
/^Diff in / { out("  " $0); next }
END { if (n > max) print "  ... " (n - max) " more line(s): see the log" }
