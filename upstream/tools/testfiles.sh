#!/bin/bash
# Symbols/types/instantiations on the PRs' tests and the new tests: tsgo builds vs tsrs switch settings.
# BIN: directory with the ref-base / ref-prs / ref-L5 tsgo builds (see ../README.md); TF: directory holding the five test files.
B=${BIN:?set BIN}; R=${TSRS:?set TSRS to a tsrs release binary}
cd ${TF:?set TF}
OFF="TSRS_LAZY_TUPLES=0 TSRS_LAZY_EMPTY=0 TSRS_LAZY_UNMATCHED=0 TSRS_LAZY_PROP_CACHE=0 TSRS_LAZY_COND_MAPPER=0"
c(){ "$@" $args 2>&1 | awk '/^Symbols:/{s=$2}/^Types:/{t=$2}/^Instantiations:/{i=$2}END{printf "%s/%s/%s", s,t,i}'; }
printf "| test | tsgo b85298b6 | tsrs opt-out | tsgo +PRs | tsrs PRs only | tsgo +PRs +stack | tsrs default |\n|---|---|---|---|---|---|---|\n"
for f in instantiatedReferenceLazyMembers mappedTypeLazyMembers tupleLazyMembers lazyMembersEmptyObjectType lazyMembersUnmatchedProperties; do
  lib=""; [ $f = mappedTypeLazyMembers ] && lib="--lib esnext"
  args="--strict --target esnext --noEmit $lib --extendedDiagnostics --singleThreaded $f.ts"
  printf "| %s | %s | %s | %s | %s | %s | %s |\n" $f "$(c $B/ref-base)" "$(c env TSRS_LAZY_MEMBERS=0 $R)" "$(c $B/ref-prs)" "$(c env $OFF $R)" "$(c $B/ref-L5)" "$(c $R)"
done
