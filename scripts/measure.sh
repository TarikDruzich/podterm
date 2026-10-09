#!/usr/bin/env bash
# Sample CPU over a fixed window instead of using lifetime-average ps %CPU.
set -euo pipefail

usage() {
  echo 'Usage: scripts/measure.sh rust|python [seconds, default 8]'
  echo 'Run the application in another terminal first; try idle and playing separately.'
  exit 2
}
[[ $# -ge 1 && $# -le 2 ]] || usage
mode="$1"
seconds="${2:-8}"
[[ "$seconds" =~ ^[1-9][0-9]*$ ]] || usage
(( seconds <= 60 )) || usage

case "$mode" in
  rust)
    pid="$(pgrep -nx podterm-rs || true)"
    ;;
  python)
    pid="$(ps -eo pid=,comm=,args= | awk '$2 ~ /^python/ && /podterm[.]py/ {print $1}' | tail -n 1)"
    ;;
  *) usage ;;
esac
[[ -n "${pid:-}" && -r "/proc/$pid/stat" ]] || { echo "No running $mode podterm process found." >&2; exit 1; }
mpv_pid="$(pgrep -nx mpv || true)"
clk="$(getconf CLK_TCK)"

ticks() {
  local line remaining
  IFS= read -r line < "/proc/$1/stat" || return 1
  remaining="${line##*) }"
  # After 'pid (comm)', positional field 1 is Linux /proc stat field 3.
  local -a fields
  read -r -a fields <<< "$remaining"
  echo "$(( ${fields[11]} + ${fields[12]} ))"
}
rss_kib() { awk '/^VmRSS:/ {print $2; exit}' "/proc/$1/status" 2>/dev/null; }

app_before="$(ticks "$pid")"
mpv_before=""
if [[ -n "$mpv_pid" && -r "/proc/$mpv_pid/stat" ]]; then mpv_before="$(ticks "$mpv_pid")"; fi
start="$EPOCHREALTIME"
echo "Measuring $mode for $seconds seconds (PID $pid)..."
sleep "$seconds"
[[ -r "/proc/$pid/stat" ]] || { echo 'Application closed during measurement.' >&2; exit 1; }
app_after="$(ticks "$pid")"
app_rss="$(rss_kib "$pid")"
end="$EPOCHREALTIME"

print_line() {
  local label="$1" rss="$2" before="$3" after="$4"
  awk -v label="$label" -v rss="$rss" -v b="$before" -v a="$after" -v clk="$clk" -v elapsed="$end-$start" \
    'BEGIN {printf "%-12s %7.1f MB  %7.2f%% CPU\n", label, rss/1024, (a-b)*100/(clk*elapsed)}'
}
print_line "$mode" "$app_rss" "$app_before" "$app_after"
if [[ -n "$mpv_before" && -r "/proc/$mpv_pid/stat" ]]; then
  mpv_after="$(ticks "$mpv_pid")"
  mpv_rss="$(rss_kib "$mpv_pid")"
  print_line mpv "$mpv_rss" "$mpv_before" "$mpv_after"
  awk -v a="$app_rss" -v b="$mpv_rss" 'BEGIN {printf "Total RSS    %7.1f MB\n", (a+b)/1024}'
fi
printf '\nRSS is per-process resident memory; totals can double-count shared pages.\n'
