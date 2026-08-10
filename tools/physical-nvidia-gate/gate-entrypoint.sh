#!/bin/sh
set -eu

nvidia-smi -L

case "${BURD_GATE_MODE:-}" in
    report)
        exit 0
        ;;
    stubborn)
        trap '' TERM INT
        while :; do
            sleep 1
        done
        ;;
    *)
        echo "invalid physical NVIDIA gate mode" >&2
        exit 64
        ;;
esac
