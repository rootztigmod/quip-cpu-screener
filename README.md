# quip-cpu-screener (experiment)

Goal: move the CPU salt screening of a Quip miner off the hub and onto the machines that already have idle cores, so that
screening capacity grows with every machine added. Nothing here is wired into a miner yet.

## Step 1: can a standalone program regenerate the hub's salt instances exactly?  PASSED
`gen-check` reads a family dump written by the hub (a small hook that writes the lease's descriptor parameters, topology and
a few real nonce/digest pairs on its first lease) and regenerates those instances with the public generator crates.
Result: 6 of 6 real salts reproduce exactly (instance digest and all 41205 couplings).

## Step 2: what does one salt cost on a CPU core?
`bench` generates N salts and scores each with the 20-number structural screen (ported from the solver, see NOTICE).
Single thread on a desktop core: about 22 ms to generate plus about 13 ms to screen, roughly 28 salts/s per thread.
On a shared 30-vCPU GPU box with the GPU worker running: 24 threads gave about 330 salts/s.

## Usage
    cargo build --release --offline
    ./target/release/gen-check family_dump.json
    ./target/release/bench family_dump.json model.json 960 24     # salts, threads

## License
AGPL-3.0-or-later (the screen is ported from the AGPL solver). The generator crates it links are Apache-2.0.
