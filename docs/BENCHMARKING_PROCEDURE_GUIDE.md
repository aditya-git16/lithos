# Performance Benchmarking Procedure

This guide defines the required process for performance validation of:

- every new hot-path function
- the full Obsidian -> Onyx pipeline end-to-end

Use this together with `/Users/adityaanand/dev/lithos/perf/PERF_GUIDE.md` (which explains chart interpretation and report contents).

## Scope

Run this procedure for any change that touches parsing, IPC, market-state updates, or any code path in the data pipeline.

## Architecture

All hot-path micro-benchmarks live in **one** criterion bench file (`bench_hot_path.rs`). Individual step times and e2e times are measured in the same run for coherent numbers. `perf_report` reads the criterion JSON and adds cross-thread e2e pipeline and soak measurements (which criterion can't do).

```
criterion (bench_hot_path.rs)  →  target/criterion/**/estimates.json
                                              ↓
perf_report (binary)           →  reads criterion JSON + runs e2e/soak  →  results/*_report.json
                                              ↓
plot_results.py                →  reads report JSON  →  results/plots/*.png
```

## Benchmark Locations

- **Criterion hot-path benchmarks** (individual steps + e2e per path):
  `/Users/adityaanand/dev/lithos/perf/benches/bench_hot_path.rs`
- **Other criterion benchmarks** (parsing, broadcast, market state, timestamps):
  `/Users/adityaanand/dev/lithos/perf/benches/`
- **Cross-thread e2e + soak report**:
  `/Users/adityaanand/dev/lithos/perf/src/bin/perf_report.rs`
- **Shared helpers** (corpus generation, criterion JSON reader, display):
  `/Users/adityaanand/dev/lithos/perf/src/lib.rs`
- **Bench registration**:
  `/Users/adityaanand/dev/lithos/perf/Cargo.toml`
- **Runner script**:
  `/Users/adityaanand/dev/lithos/perf/scripts/run_perf.sh`

## Required Workflow (From Now On)

### 1. Add a criterion benchmark for every new hot-path function

Add it to the appropriate group in `bench_hot_path.rs`:

- **Obsidian path functions** → `bench_obsidian()` group
- **Onyx path functions** → `bench_onyx()` group

Use realistic inputs (corpus-based where possible) and `black_box(...)` on inputs/outputs.

Template:

```rust
// Inside bench_obsidian() or bench_onyx()
group.bench_function("new_function", |b| {
    b.iter(|| {
        black_box(new_function(black_box(input)));
    });
});
```

### 2. Keep e2e benchmarks accurate

- If the new function is part of an existing path, update the e2e benchmark in `bench_hot_path.rs` (`process_text` for obsidian, `poll_event` for onyx) so it includes the new work.
- If pipeline behavior changed, update the cross-thread path in `perf_report.rs` `section_pipeline_summary()`.

### 3. Update perf_report criterion display

Add the new benchmark's criterion key to `section_criterion_paths()` in `perf_report.rs` so it appears in the report output:

```rust
let obs_steps: &[(&str, &str)] = &[
    // ... existing steps ...
    ("obsidian/new_function", "new_function"),  // add here
];
```

### 4. Update plot_results.py

Add the new criterion key to the relevant chart functions (`plot_stage_breakdown`, `plot_pipeline_waterfall`, `plot_component_matrix`) so it appears in charts.

### 5. Run the full suite (required)

```bash
bash perf/scripts/run_perf.sh perf
```

This runs: `build → criterion → report → plots` in the correct order (criterion first so perf_report can read its JSON).

For criterion-only iteration during development:

```bash
cargo bench -p lithos-perf --bench bench_hot_path
```

Optional flamegraph:

```bash
bash perf/scripts/run_perf.sh perf --flamegraph
```

### 6. Verify outputs

- Criterion output exists in: `target/criterion/`
- JSON report exists in: `perf/results/*_report.json`
- Report JSON includes `criterion_benchmarks` array with the new function
- Charts are generated in: `perf/results/plots/`

### 7. Validate key report sections

- Confirm the new function appears in the criterion path display (OBSIDIAN HOT PATH or ONYX HOT PATH section).
- Confirm e2e times and overhead analysis are present for both paths.
- Confirm `pipeline e2e` cross-thread measurement is present with 256 symbols.
- Confirm `soak_latency` is present.
- Check overrun/filter counters in the pipeline summary before signing off.

### 8. PR checklist (required)

- [ ] Criterion benchmark added in `bench_hot_path.rs` for each new hot-path function
- [ ] E2e benchmark updated (if path changed)
- [ ] Criterion key added to `perf_report.rs` display
- [ ] Criterion key added to `plot_results.py` charts
- [ ] `perf` mode executed successfully (`bash perf/scripts/run_perf.sh perf`)
- [ ] PR includes before/after median and p99 for changed stages and `pipeline e2e`

## Standard Command Sequence

Use this for full perf validation:

```bash
bash perf/scripts/run_perf.sh perf
```

For faster iteration on micro-benchmarks only:

```bash
cargo bench -p lithos-perf --bench bench_hot_path
```
