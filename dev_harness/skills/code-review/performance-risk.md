# Performance Risk Review

## Objective

Identify whether the change introduces avoidable latency, memory growth, CPU overhead, I/O amplification, or scalability constraints.

## Review checks

- Check new loops, recursion, allocations, clones, buffering, and collection growth for boundedness.
- Check whether synchronous work is added to latency-sensitive or UI-facing paths.
- Check whether caching, batching, streaming, or pagination is required but absent.
- Check whether repeated file, network, database, or model calls can scale with user input size.
- Check whether new background work has cancellation, timeout, and backpressure behavior.
- Check whether performance-sensitive behavior requires benchmark or evaluation material under `dev_harness/eval/`.

## Finding format

- Resource affected.
- Scaling variable.
- Worst-case behavior.
- Measurement or mitigation required.
