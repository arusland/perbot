use std::hint::black_box;
use std::time::Instant;

use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime};

use perbot::scheduler::calc_next_at;

/// Benchmark for [`calc_next_at`]: steps the recurring pattern
/// `"15:30 every 3 days"` through every occurrence across a 100-year horizon
/// and reports the per-call cost of the scheduler's repetition math.
///
/// The event is fed back into `calc_next_at` each iteration (carrying its
/// `next_datetime` forward) — exactly how the bot reschedules a fired event —
/// so successive calls advance by the 3-day interval rather than recomputing
/// the first occurrence from scratch.
fn main() {
    const PATTERN: &str = "15:30 every 3 days";

    // `parse` rejects a body-less input, so append a reminder body; only the
    // time/repetition fields drive the scheduler under test.
    let parsed = perbot::parser::parse(&format!("{PATTERN} reminder"), &perbot::locale::EN)
        .expect("pattern should parse into an event");

    let start = NaiveDateTime::new(
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
    );
    let end = NaiveDateTime::new(
        NaiveDate::from_ymd_opt(2126, 1, 1).unwrap(),
        NaiveTime::from_hms_opt(0, 0, 0).unwrap(),
    );
    let years = (end - start).num_days() as f64 / 365.25;

    // Warm-up: a few hundred steps, results discarded.
    {
        let mut ev = parsed.clone();
        let mut cur = start;
        for _ in 0..500 {
            ev = calc_next_at(ev, cur);
            let next = ev
                .next_datetime
                .expect("recurring event always reschedules");
            cur = next + Duration::seconds(1);
        }
    }

    // Timed run: step from `start` until the next fire reaches `end`.
    let mut ev = parsed.clone();
    let mut cur = start;
    let mut iterations: u64 = 0;
    let timer = Instant::now();
    loop {
        ev = calc_next_at(ev, cur);
        let next = ev
            .next_datetime
            .expect("recurring event always reschedules");
        black_box(&ev);
        iterations += 1;
        cur = next + Duration::seconds(1);
        if next >= end {
            break;
        }
    }
    let elapsed = timer.elapsed();

    let total_ms = elapsed.as_secs_f64() * 1000.0;
    let ns_each = elapsed.as_nanos() as f64 / iterations as f64;
    let calls_per_sec = iterations as f64 / elapsed.as_secs_f64();

    println!("=== Scheduler Benchmark (calc_next_at) ===");
    println!("Pattern:          \"{PATTERN}\"");
    println!("Span:             {years:.1} years ({start} -> {end})");
    println!("Occurrences:      {iterations}");
    println!("Total:            {total_ms:.2} ms");
    println!("Per call:         {ns_each:.1} ns");
    println!("Throughput:       {calls_per_sec:.0} calls/sec");
    println!("==========================================");
}
