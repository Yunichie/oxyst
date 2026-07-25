# Oxyst benchmarks

This directory contains performance benchmark for Oxyst's
deterministic, public operation boundaries.

## Requirements

- Python 3.8 or newer
- Cargo and the Rust toolchain required by the workspace

## Run

From the repository root on macOS or Linux:

```bash
python3 benchmarks/run.py
```

On Windows:

```bash
py -3 benchmarks/run.py
```

To run a Criterion filter while developing the suite:

```bash
python3 benchmarks/run.py --filter document
```

To regenerate the summaries for an existing result:

```bash
python3 benchmarks/run.py --summarize benchmarks/results/20260725-081822
```

Each execution writes an immutable UTC-stamped directory under `results/`.
It contains the full Criterion output and raw samples, machine-readable and
human-readable summaries, and enough environment metadata to identify the
source revision and test machine.

## Method

The suite pins Criterion.rs 0.8.2 and uses its standard release-mode,
wall-clock methodology:

- 3 seconds of warm-up per benchmark;
- 100 measured samples over a nominal 5-second measurement period;
- 100,000 bootstrap resamples and 95% confidence intervals;
- automatic sampling for normal operations;
- 30 flat samples over 10 seconds for slow compiler startup;
- `std::hint::black_box` to keep observable work from being optimized away;
- `iter_batched` to exclude large document setup from edit timings;
- byte or element throughput wherever a workload has a meaningful size.

Criterion may extend the nominal measurement period when an operation is slow.
The raw samples and Criterion baseline are retained so later runs can make
statistical comparisons.

## Coverage

| Area | Measured operations |
|---|---|
| Configuration | construct defaults; read, parse, validate, and merge an explicit file |
| Document | create and serialize 256 KiB; middle insert and paste; large selection deletion; undo |
| Cursor | grapheme and word movement over 1,000 operations |
| Compiler startup | initialize the Typst world/fonts and compile a small document |
| Compiler steady state | unchanged and single-edit compilation; dependency invalidation/import; invalid-source diagnostics; snapshot; word count |
| Synchronization | source cursor to preview position and preview position to source |
| Rendering | manifest, full and single-page rasterization, and warm-cache reuse |
| Export | end-to-end PDF, PNG, and SVG encoding plus file replacement |

Terminal input latency, screen refresh rate, clipboard access, filesystem
watch notifications, and background task scheduling are intentionally excluded.
They depend on interactive devices, OS scheduling, terminal capabilities, or
race timing and do not have a stable in-process workload. Theme lookups and
simple accessors are excluded because they are constant-time operations below
meaningful application latency. The public core operations above are the
repeatable workloads that dominate user-visible work.

The fixtures are checked in and deterministic. The editor workload is generated
from a fixed line until it reaches 256 KiB; render and compiler workloads use the
checked-in small, imported, invalid, and four-page representative Typst sources.
