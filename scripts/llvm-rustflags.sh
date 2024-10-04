#!/bin/sh

LLVM_PREFIX=$1

RUSTFLAGS="-L native=$LLVM_PREFIX"
RUSTFLAGS+=$(find $LLVM_PREFIX -type f -name "*.a" -printf '%f\n' | \
    sed -e 's/^lib//' -e 's/\.a$//' | \
    sed 's/^/-l static=/' | paste -sd ' ')

echo $RUSTFLAGS
