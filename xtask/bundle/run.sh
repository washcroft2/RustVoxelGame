#!/bin/sh
cd "$(dirname "$0")" || exit 1
chmod +x spike-* 2>/dev/null

echo
echo "This test takes about 10 minutes. Please leave this window open until it says Done."
echo

REPORT=voxel-report.txt
{
    echo "Voxel game test report"
    date -u
    echo
    echo "=== Computer ==="
    uname -a
    cat /etc/os-release 2>/dev/null
    lscpu 2>/dev/null
    free -h 2>/dev/null
    lspci 2>/dev/null | grep -iE 'vga|3d|display'
} > "$REPORT" 2>&1

for p in spike-gpu spike-wasmtime spike-mesher spike-redb spike-circuit spike-quinn; do
    echo "Running $p ..."
    {
        echo
        echo "=== $p ==="
        "./$p"
        echo "exit code: $?"
    } >> "$REPORT" 2>&1
done

echo
echo "Done. Please send the file voxel-report.txt from this folder to whoever sent you this."
