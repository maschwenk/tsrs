#!/bin/bash
# interleaved --noCheck runs at several rayon pool sizes
T=$HOME/Developer/tsrs-work/wt/persisted-fe/target/release/tsrs
cd $HOME/Developer/tsrs-work/owner-clone/apps/olympus
for round in 1 2 3; do
  for n in 1 4 8 18; do
    while [ "$(uptime | sed 's/.*averages: //' | cut -d' ' -f1 | cut -d. -f1)" -gt 40 ]; do sleep 5; done
    out=$(RAYON_NUM_THREADS=$n /usr/bin/time -l $T -p . --noEmit --noCheck --extendedDiagnostics --pretty false --incremental false 2>&1)
    parse=$(echo "$out" | grep "^Parse time" | awk '{print $3}')
    bind=$(echo "$out" | grep "^Bind time" | awk '{print $3}')
    total=$(echo "$out" | grep "^Total time" | awk '{print $3}')
    ppr=$(echo "$out" | grep "parallel parse + resolve" | awk '{print $NF}')
    seq=$(echo "$out" | grep "sequential load" | awk '{print $NF}')
    rss=$(echo "$out" | grep "maximum resident" | awk '{printf "%.2f", $1/2^30}')
    load=$(uptime | sed 's/.*averages: //' | cut -d' ' -f1)
    echo "round=$round threads=$n parse=$parse ppr=$ppr seq=$seq bind=$bind total=$total rss=$rss load=$load"
  done
done
