#!/bin/bash
# Border resize test loop.
#
# For each iteration: center the window, pick a random edge (top/bottom/
# left/right), move the real pointer to that edge's band, drag it, then
# verify the window geometry changed as expected.
#
# Positioning uses the ABSOLUTE uinput device (denia-input abs/adrag):
# absolute coordinates bypass libinput pointer acceleration entirely and
# map 1:1 onto the logical desktop (verified ±1px), so no feedback loop
# is needed and the top band inside the shell title bar — where cursor
# broadcasts stop — is reachable exactly.
#
# Usage: border-resize-test.sh [ITERATIONS] [SHOT_DIR]
# Env:
#   NO_RESET=1     keep the window state between iterations (no shrink)
#   FIXED_ORDER=1  cycle top/bottom/left/right instead of random edges
set -u

API="http://127.0.0.1:17894/api/ui"
ITER="${1:-5}"
SHOT_DIR="${2:-/tmp/denial-resize-tests}"
DENIA_INPUT="${DENIA_INPUT:-$HOME/.local/bin/denia-input}"
GRIM_ENV="WAYLAND_DISPLAY=wayland-1 XDG_RUNTIME_DIR=/run/user/1000"
mkdir -p "$SHOT_DIR"

ui() { curl -fsS -X POST "$API" -d "$1" 2>/dev/null; }

jget() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)" 2>/dev/null; }

geo_vals() {
  ui '{"key":"window","action":"geometry"}' | python3 -c "
import json,sys
d=json.load(sys.stdin)['rect']
print(d['x'], d['y'], d['w'], d['h'])"
}

# Absolute pointer positioning: 1:1, no acceleration, no feedback loop.
pointer() {
  "$DENIA_INPUT" abs "$1" "$2"
  sleep 0.3
}

# Absolute drag: press at (from_x,from_y), glide to (to_x,to_y), release.
drag_to() {
  DENIA_INPUT_FROM="$1,$2" "$DENIA_INPUT" adrag "$3" "$4"
  sleep 0.9
}

# Rebase: jump into a known live point (top-left corner of the client area
# of the centered window, re-read after the window is centered).
rebase_pointer() {
  pointer 960 400
}

# Make sure a test window exists and fits the screen; shrink it via real
# border resize grabs if it outgrew the limits (a relaunch would just be
# restored to the remembered large size by the client).
ensure_window() {
  local vals
  vals="$(geo_vals)"
  if [[ -z "$vals" || "$(echo "$vals" | awk '{print $3}')" == "" ]]; then
    ui '{"key":"apps","action":"launch","args":{"id":"kitty.desktop"}}' >/dev/null
    sleep 3
    vals="$(geo_vals)"
  fi
  local ww wh
  ww="$(echo "$vals" | awk '{print $3}')"
  wh="$(echo "$vals" | awk '{print $4}')"
  if [[ "${NO_RESET:-0}" != "1" ]] && (( ww > 1100 || wh > 750 )); then
    shrink_window
  fi
}

# Shrink the window back to ~800x560 using native border resize grabs
# (right edge then bottom edge, iterating until inside the limits).
shrink_window() {
  local wx wy ww wh t
  for ((t = 0; t < 8; t++)); do
    read wx wy ww wh < <(geo_vals)
    if (( ww <= 810 && wh <= 570 )); then
      break
    fi
    if (( ww > 810 )); then
      # Right edge: press 6px inside the content edge, drag left to 800 wide.
      drag_to $((wx + ww - 6)) $((wy + 60)) $((wx + 800 - 6)) $((wy + 60))
    fi
    read wx wy ww wh < <(geo_vals)
    if (( wh > 570 )); then
      # Bottom edge: press 6px above the content edge, drag up to 560 tall.
      drag_to $((wx + ww / 2)) $((wy + wh - 6)) $((wx + ww / 2)) $((wy + 560 - 6))
    fi
  done
  ui '{"key":"window","action":"center"}' >/dev/null
  sleep 0.4
}

pass=0
fail=0
results=()
FIXED_EDGES=(top bottom left right)

rebase_pointer
ensure_window

for ((i = 1; i <= ITER; i++)); do
  echo "=== 测试 $i/$ITER ==="

  ensure_window

  centered="$(ui '{"key":"window","action":"center"}' )"
  wx="$(echo "$centered" | jget "d['rect']['x']")"
  wy="$(echo "$centered" | jget "d['rect']['y']")"
  ww="$(echo "$centered" | jget "d['rect']['w']")"
  wh="$(echo "$centered" | jget "d['rect']['h']")"
  if [[ -z "$wx" || -z "$ww" ]]; then
    echo "!! 无法居中窗口（没有可测试的窗口？）"
    fail=$((fail + 1))
    results+=("$i: SKIP (no window)")
    continue
  fi
  echo "  居中后: ${wx},${wy} ${ww}x${wh}"
  sleep 0.4

  if [[ "${FIXED_ORDER:-0}" == "1" ]]; then
    edge="${FIXED_EDGES[(((i - 1)) % ${#FIXED_EDGES[@]})]}"
  else
    edges=(top bottom left right)
    edge="${edges[$((RANDOM % 4))]}"
  fi

  case "$edge" in
    top)    tx=$((wx + ww / 2)); ty=$((wy + 6)) ;;
    bottom) tx=$((wx + ww / 2)); ty=$((wy + wh - 6)) ;;
    left)   tx=$((wx + 6));      ty=$((wy + wh / 2)) ;;
    right)  tx=$((wx + ww - 6)); ty=$((wy + wh / 2)) ;;
  esac
  echo "  选边: $edge  (目标点 ${tx},${ty})"

  # Move into the edge band (absolute; works for the top band too, which
  # sits inside the shell-owned title bar with no cursor broadcast).
  pointer "$tx" "$ty"

  d=$((150 + RANDOM % 150))
  # Clamp to the space left on screen so the drag can never push the
  # window past the display edge (which would break the exact expected
  # geometry check).
  case "$edge" in
    top)    maxd=$((ty - 6)) ;;
    bottom) maxd=$((1274 - ty)) ;;
    left)   maxd=$((tx - 6)) ;;
    right)  maxd=$((1914 - tx)) ;;
  esac
  ((d > maxd)) && d=$maxd
  # Respect the 40px floor only when the screen still has that much room;
  # near the display edge maxd wins or the drag target would be clamped
  # by the compositor and break the exact expected-geometry check.
  if ((maxd >= 40 && d < 40)); then d=40; fi
  ((d > maxd)) && d=$maxd
  case "$edge" in
    top)    ex=$tx;            ey=$((ty - d)); m1x=$tx; m1y=$((ty - d / 3)); m2x=$tx; m2y=$((ty - 2 * d / 3)) ;;
    bottom) ex=$tx;            ey=$((ty + d)); m1x=$tx; m1y=$((ty + d / 3)); m2x=$tx; m2y=$((ty + 2 * d / 3)) ;;
    left)   ex=$((tx - d));    ey=$ty; m1x=$((tx - d / 3)); m1y=$ty; m2x=$((tx - 2 * d / 3)); m2y=$ty ;;
    right)  ex=$((tx + d));    ey=$ty; m1x=$((tx + d / 3)); m1y=$ty; m2x=$((tx + 2 * d / 3)); m2y=$ty ;;
  esac
  # Expected final geometry (resize keeps the press-point offset, so the
  # affected dimension grows by exactly the drag distance).
  exp_w=$ww; exp_h=$wh; exp_x=$wx; exp_y=$wy
  case "$edge" in
    top)    exp_h=$((wh + d)); exp_y=$((wy - d)) ;;
    bottom) exp_h=$((wh + d)) ;;
    left)   exp_w=$((ww + d)); exp_x=$((wx - d)) ;;
    right)  exp_w=$((ww + d)) ;;
  esac
  echo "  闭环: press(${tx},${ty}) -> (${m1x},${m1y}) -> (${m2x},${m2y}) -> release(${ex},${ey})  预期 ${exp_w}x${exp_h}@${exp_x},${exp_y}"

  # Phased drag in the background so the harness can observe the geometry
  # at press / mid-drag / release / post-release.
  "$DENIA_INPUT" pdrag "$tx,$ty" "$m1x,$m1y" "$m2x,$m2y" "$ex,$ey" --ms 500 &
  pd_pid=$!
  sleep 0.9
  g_press="$(geo_vals)"   # after press, before any motion
  env $GRIM_ENV grim -o HDMI-A-2 "$SHOT_DIR/test-$i-$edge-press.png" 2>/dev/null
  sleep 0.7
  g_mid="$(geo_vals)"     # mid-drag (~1/3 of the way)
  wait "$pd_pid"
  g_end="$(geo_vals)"     # after release
  # Move the pointer away: a released grab must not keep affecting the window.
  "$DENIA_INPUT" abs 960 100
  sleep 0.8
  g_stable="$(geo_vals)"  # post-release stability
  env $GRIM_ENV grim -o HDMI-A-2 "$SHOT_DIR/test-$i-$edge-end.png" 2>/dev/null

  read pwx pwy pww pwh < <(echo "$g_press")
  read mwx mwy mww mwh < <(echo "$g_mid")
  read ewx ewy eww ewh < <(echo "$g_end")
  read swx swy sww swh < <(echo "$g_stable")

  echo "  按下后: ${pwx},${pwy} ${pww}x${pwh} | 拖动中: ${mwx},${mwy} ${mww}x${mwh} | 松开后: ${ewx},${ewy} ${eww}x${ewh} | 移走后: ${swx},${swy} ${sww}x${swh}"

  # ---- Closed-loop verdict ----
  ok_press=1; ok_mid=1; ok_end=1; ok_stable=1
  # 1. Press must not change the geometry yet.
  if ((pww != ww || pwh != wh || pwx != wx || pwy != wy)); then ok_press=0; fi
  # 2. Mid-drag must have started growing the affected dimension (the
  # 1/3-way observation can be only a few px when the window fills the
  # screen, so require any positive growth, not a fixed margin).
  case "$edge" in
    top|bottom) ((mwh > wh)) || ok_mid=0 ;;
    left|right) ((mww > ww)) || ok_mid=0 ;;
  esac
  # 3. After release the geometry must equal the expected value (±3px).
  if ((eww < exp_w - 3 || eww > exp_w + 3 || ewh < exp_h - 3 || ewh > exp_h + 3 ||
       ewx < exp_x - 3 || ewx > exp_x + 3 || ewy < exp_y - 3 || ewy > exp_y + 3)); then
    ok_end=0
  fi
  # 4. Moving the pointer away must not change the geometry.
  if ((sww != eww || swh != ewh || swx != ewx || swy != ewy)); then ok_stable=0; fi

  if ((ok_press && ok_mid && ok_end && ok_stable)); then
    echo "  ✅ PASS: $edge 边完整闭环 (${ww}x${wh} -> ${eww}x${ewh})"
    pass=$((pass + 1))
    results+=("$i: PASS ($edge)")
  else
    echo "  ❌ FAIL: $edge 边闭环失败"
    ((ok_press)) || echo "     - 按下后几何已变 (${pww}x${pwh} != ${ww}x${wh})"
    ((ok_mid))   || echo "     - 拖动中未部分增长 (${mww}x${mwh})"
    ((ok_end))   || echo "     - 松开后尺寸不符预期 ${exp_w}x${exp_h}@${exp_x},${exp_y} (实际 ${eww}x${ewh}@${ewx},${ewy})"
    ((ok_stable)) || echo "     - 移走指针后几何仍变 (${sww}x${swh} != ${eww}x${ewh})"
    fail=$((fail + 1))
    results+=("$i: FAIL ($edge)")
  fi

  env $GRIM_ENV grim -o HDMI-A-2 "$SHOT_DIR/test-$i-$edge.png" 2>/dev/null
done

echo
echo "=== 汇总: PASS=$pass FAIL=$fail (共 $ITER) ==="
for r in "${results[@]}"; do echo "  $r"; done
echo "截图目录: $SHOT_DIR"
