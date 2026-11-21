# Hub side

The hub (the Quip CUDA native two-pass solver) gets a small client for the broker. It is delivered as a patch against the
solver source used by the hub, `hub/0001-remote-screeners.patch` (AGPL-3.0-or-later, same as the solver).

Off by default. With `QUIP_SCREENERS` unset the hub behaves exactly as before.

| variable | meaning | default |
|---|---|---|
| `QUIP_SCREENERS` | broker address as seen from the hub (host:port) | unset (off) |
| `QUIP_SCREENER_TOKEN` | shared token (use a file, never the command line) | none |
| `QUIP_SCREENER_FEEDERS` | threads that each keep one screener batch in flight | 6 |
| `QUIP_SCREENER_KEEP` | fraction of every batch a screener returns | 0.05 |
| `QUIP_SCREENER_MAX_BATCH` | largest batch handed to a screener (salts at risk if one dies) | 2048 |

How it behaves: a feeder asks the broker for a free screener; if there is none, it takes no salts and nothing changes. With one,
it takes a block of salt indices from the lease, sends their nonces, rebuilds the returned candidates itself and re-scores them
(it never trusts a screener's score), puts them in the same best-first pool the local screen feeds, and tells the coordinator
that every other salt of the block was screened out. If a batch fails, its salts are skipped, never lost silently.

## Broker
`broker --screeners ADDR:PORT --hub ADDR:PORT` with `QUIP_SCREENER_TOKEN` set. Bind the screener address to the private
network interface (for example the Tailscale address) and the hub address to whatever the hub container reaches the host on.
The broker spot-checks the first two scores of every reply and drops a screener that fails.

## Screener
`QUIP_SCREENER_TOKEN=... screener --hub BROKER_ADDR:PORT [--threads N] [--name NAME]`; see `scripts/gcp_startup.sh` for a
complete VM bootstrap.
