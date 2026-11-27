# Benchmarks — slipway

Measured 2026-11-27 on primo: 13th Gen Intel Core i7-13620H (16 threads), 62 GiB RAM, cargo 1.100.0, debug-profile server binary driven over stdio by `benches/hot_paths.rs` (hand-rolled on `std::time`; warmup 20, measured loops as stated). Primo was under heavy parallel load during the run (other fleet work), so treat these as order-of-magnitude baselines, not lab numbers.

## Workloads

- `cold_start_discover` — spawn the server against a generated scratch config, send `server/discover`, await the correlated response; 10 spawns.
- `tools_list` — `tools/list` over one warm session; 200 iterations.
- `tool_call_slipway_status` — read-only `slipway_status` call over the same warm session; 200 iterations.

## Results (2026-11-27)

| workload | iters | total | per-op | ops/s |
|---|---|---|---|---|
| cold_start_discover | 10 | 318.2 ms | 31.82 ms | 31 |
| tools_list | 200 | 81.2 ms | 405.9 µs | 2,464 |
| tool_call_slipway_status | 200 | 51.1 ms | 255.6 µs | 3,912 |

## Example

`examples/stdio_session.rs` — one modern-era session (discover, tools/list, `slipway_status`) against a throwaway generated config; prints the 15 tool names and the status summary. Run: `cargo run --example stdio_session`.
