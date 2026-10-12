#!/bin/sh
# Terminal child for the notifications scenario.
esc=$(printf '\033'); bel=$(printf '\007')
sleep 2
printf '%s]9;osc9 body%s' "$esc" "$bel"; sleep 1.2
printf '%s]777;notify;Title777;Body777%s' "$esc" "$bel"; sleep 1.2
printf '%s]777;notify;;OnlyBody%s' "$esc" "$bel"; sleep 1.2
printf '%s]777;notify;Only title;%s' "$esc" "$bel"; sleep 1.2
printf '%s]9;<b>markup</b> & "quotes" \303\251\342\202\254%s' "$esc" "$bel"; sleep 1.2
printf '%s]9;osc9 body%s' "$esc" "$bel"; sleep 1.2
printf '%s]777;notify;semi;colon;body%s' "$esc" "$bel"; sleep 1.2
# command finished (OSC 133 prompt marks): C then D;3 after >1s
printf '%s]133;A%s%s]133;B%s%s]133;C%s' "$esc" "$bel" "$esc" "$bel" "$esc" "$bel"; sleep 2
printf '%s]133;D;3%s' "$esc" "$bel"; sleep 1.5
printf '%s]133;A%s%s]133;B%s%s]133;C%s' "$esc" "$bel" "$esc" "$bel" "$esc" "$bel"; sleep 2
printf '%s]133;D;0%s' "$esc" "$bel"
# wait for the harness to ask for one more notification
while [ ! -e "$FLAG2" ]; do sleep 0.3; done
printf '%s]9;after reload%s' "$esc" "$bel"
sleep 60
