# Phase 0: machine specs

Section 2 asks for the real specs of every machine that will play, replacing the placeholder
reference hardware. So far only one is recorded, and it is the machine the spikes ran on.

## This machine (assumed to be one of the family's Linux PCs; not confirmed)

| | |
|---|---|
| CPU | AMD Ryzen 7 5700X, 8 cores / 16 threads |
| RAM | 31 GB |
| GPU | NVIDIA GeForce RTX 2060 SUPER (Vulkan 1.4, driver 615.78.08) |
| OS | CachyOS Linux, kernel 7.3 |
| Disk | NVMe, btrfs (home partition 88% full) |
| Rust | 1.98.1 |
| Display | not recorded |

Against section 2's placeholder "A": CPU matches (8 cores), RAM is higher (31 GB vs 16 GB), and the
GPU is in the same class as the assumed RTX 3060 though somewhat slower. The numbers in
`docs/phase0/*.md` were all measured here.

## Still needed (from the people who own them)

- The Windows PC: CPU, RAM, GPU and driver, display resolution and refresh rate.
- The MacBook: model, chip (Apple silicon or Intel), RAM, macOS version.
- The weakest machine anyone will actually use (section 2 "C").
- Whether the Linux PC above is the family's main PC, and its display.

`spike-gpu` writes the adapter and feature list to `gpu-report.txt`, which gives the GPU half of
this for each machine without asking anyone to read a spec sheet.
