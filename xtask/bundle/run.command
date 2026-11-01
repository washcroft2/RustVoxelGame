#!/bin/bash
# Double-click in Finder: opens Terminal and runs the tests.
cd "$(dirname "$0")" || exit 1
# Files from a download carry a quarantine flag; clear it so the programs can run.
xattr -dr com.apple.quarantine . 2>/dev/null
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
    sw_vers
    uname -m
    sysctl -n machdep.cpu.brand_string 2>/dev/null
    sysctl -n hw.model
    echo "memory bytes: $(sysctl -n hw.memsize)"
    system_profiler SPDisplaysDataType 2>/dev/null
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
echo
read -r -p "Press Enter to close this window. " _
