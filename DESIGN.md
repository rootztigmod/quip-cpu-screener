# Design: universal CPU screeners

```
 CPU VPS (any number)               laptop host                       hub (node container)
 +----------------+  Tailscale   +--------------+   host gateway   +-------------------+
 | screener       |------------->| broker       |<-----------------| solver (remote    |
 | gen + CPU screen|  token auth  | (this repo)  |  batches/passes  |  screen source)   |
 +----------------+              +--------------+                  +-------------------+
```

* The **screener** dials the **broker**, authenticates with a token, and is sent everything it needs (descriptor
  parameters, topology, the screen model). It needs no configuration. It pulls batches of salt nonces, generates each
  instance with the public generator crates, scores it with the CPU screen and returns the best few per batch.
* The **broker** runs next to the hub, listens on the Tailscale address, hands out batches sized from each screener's measured
  speed, re-issues a batch if a screener disappears, and spot-checks results by recomputing a few scores itself.
* The **hub** treats the broker as one more remote source of candidates (a nonce and a score). It rebuilds the full instance for
  the few candidates it solves and re-scores them, so a screener can never fake a result; it can only waste a little GPU time.

Why a broker and not a listener inside the hub: the hub container has no published ports and its compose file is owned by the
node manager; a host-side broker leaves the node untouched and keeps access control outside the solver.

## Frames
Length-prefixed (kind u8, len u32 little endian, body). Magic 0x51535231 in HELLO.

| kind | direction | body |
|---|---|---|
| 1 HELLO | screener -> broker | magic u32, version u32, token 32 B, threads u32, name (u8 length + bytes) |
| 2 WELCOME | broker -> screener | status u8 (0 ok), epoch u64, params (u32 len + bytes), nodes (u32 n + n*u32), edges (u32 m + m*(u32,u32)), model (61*f64) |
| 3 BATCH | broker -> screener | batch id u64, epoch u64, keep fraction f32, n u32, n*32 B nonces |
| 4 RESULT | screener -> broker | batch id u64, elapsed ms u32, k u32, k*(index u32, score f64) |
| 5 CANCEL | broker -> screener | batch id u64 |

Lower score is better. `index` is the position inside the batch.
