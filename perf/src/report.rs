use crate::{BenchResult, compute_stats};
use lithos_perf_recorder::{NUM_STAGES, PerfRecorder, PerfStage};

pub const STAGE_NAMES: [&str; NUM_STAGES] = [
    "ParseJson",
    "ParseNumeric",
    "BuildTob",
    "TimestampEvent",
    "Publish",
    "TryRead",
    "ProcessEvent",
    "PrefetchNext",
    "ObsidianTotal",
    "OnyxTotal",
];

pub const ALL_STAGES: [PerfStage; NUM_STAGES] = [
    PerfStage::ParseJson,
    PerfStage::ParseNumeric,
    PerfStage::BuildTob,
    PerfStage::TimestampEvent,
    PerfStage::Publish,
    PerfStage::TryRead,
    PerfStage::ProcessEvent,
    PerfStage::PrefetchNext,
    PerfStage::ObsidianTotal,
    PerfStage::OnyxTotal,
];

/// Convert PerfRecorder stage samples into BenchResults.
pub fn stage_results(recorder: &PerfRecorder) -> Vec<BenchResult> {
    let mut out = Vec::new();
    for (i, &stage) in ALL_STAGES.iter().enumerate() {
        let mut samples: Vec<u64> = recorder.samples(stage).to_vec();
        if samples.is_empty() {
            continue;
        }
        let stats = compute_stats(&mut samples);
        out.push(BenchResult {
            name: STAGE_NAMES[i].to_string(),
            unit: "ns".to_string(),
            stats,
        });
    }
    out
}

/// Obsidian stages (publisher side)
const OBSIDIAN_STAGES: [PerfStage; 6] = [
    PerfStage::ParseJson,
    PerfStage::ParseNumeric,
    PerfStage::TimestampEvent,
    PerfStage::BuildTob,
    PerfStage::Publish,
    PerfStage::ObsidianTotal,
];

/// Onyx stages (consumer side)
const ONYX_STAGES: [PerfStage; 4] = [
    PerfStage::TryRead,
    PerfStage::ProcessEvent,
    PerfStage::PrefetchNext,
    PerfStage::OnyxTotal,
];

pub fn print_stage_table(recorder: &PerfRecorder, stages: &[PerfStage], total_stage: PerfStage) {
    println!(
        "  {:<20} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}  {:>6}",
        "Stage", "p50", "p90", "p99", "p99.9", "max", "count", "% tot"
    );
    println!("  {}", "\u{2500}".repeat(88));

    let total_p50 = {
        let s = recorder.samples(total_stage);
        if s.is_empty() {
            0u64
        } else {
            let mut v = s.to_vec();
            v.sort_unstable();
            v[v.len() / 2]
        }
    };

    for &stage in stages {
        let samples = recorder.samples(stage);
        if samples.is_empty() {
            continue;
        }
        let mut v = samples.to_vec();
        let stats = compute_stats(&mut v);
        let pct = if total_p50 > 0 && stage != total_stage {
            format!("{:.0}%", stats.p50 as f64 / total_p50 as f64 * 100.0)
        } else if stage == total_stage {
            "100%".to_string()
        } else {
            "-".to_string()
        };
        let name = STAGE_NAMES[stage as usize];
        println!(
            "  {:<20} {:>8} {:>8} {:>8} {:>8} {:>8} {:>8}  {:>6}",
            name, stats.p50, stats.p90, stats.p99, stats.p999, stats.max, stats.count, pct
        );
    }
}

pub fn print_obsidian_report(recorder: &PerfRecorder) {
    println!("\n  Obsidian Per-Stage Timing:\n");
    print_stage_table(recorder, &OBSIDIAN_STAGES, PerfStage::ObsidianTotal);
}

pub fn print_onyx_report(recorder: &PerfRecorder) {
    println!("\n  Onyx Per-Stage Timing:\n");
    print_stage_table(recorder, &ONYX_STAGES, PerfStage::OnyxTotal);
}

pub fn print_analysis(obsidian: &PerfRecorder, onyx: &PerfRecorder) {
    println!("\n  Bottleneck Analysis:\n");

    let p50_of = |rec: &PerfRecorder, stage: PerfStage| -> u64 {
        let s = rec.samples(stage);
        if s.is_empty() {
            return 0;
        }
        let mut v = s.to_vec();
        v.sort_unstable();
        v[v.len() / 2]
    };

    let obs_total = p50_of(obsidian, PerfStage::ObsidianTotal);
    let onyx_total = p50_of(onyx, PerfStage::OnyxTotal);

    if obs_total > 0 {
        let parse = p50_of(obsidian, PerfStage::ParseJson);
        let numeric = p50_of(obsidian, PerfStage::ParseNumeric);
        let publish = p50_of(obsidian, PerfStage::Publish);
        println!(
            "    Obsidian total p50: {} ns (parse={} ns, numeric={} ns, publish={} ns)",
            obs_total, parse, numeric, publish
        );

        if obs_total > 0 {
            let candidates = [
                (parse, "ParseJson"),
                (numeric, "ParseNumeric"),
                (publish, "Publish"),
            ];
            let biggest = candidates.iter().max_by_key(|(v, _)| *v).unwrap();
            println!(
                "    -> Obsidian bottleneck: {} ({:.0}% of total)",
                biggest.1,
                biggest.0 as f64 / obs_total as f64 * 100.0
            );
        }
    }

    if onyx_total > 0 {
        let process = p50_of(onyx, PerfStage::ProcessEvent);
        let prefetch = p50_of(onyx, PerfStage::PrefetchNext);
        println!(
            "    Onyx total p50: {} ns (process={} ns, prefetch={} ns)",
            onyx_total, process, prefetch
        );
    }

    if obs_total > 0 && onyx_total > 0 {
        println!(
            "    Combined pipeline p50: ~{} ns (obsidian + onyx, excl. IPC transit)",
            obs_total + onyx_total
        );
    }
}
