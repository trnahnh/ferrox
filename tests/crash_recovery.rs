//! Fault-injection harness for crash consistency of the WAL + snapshot path.
//!
//! The contract under test: after any crash, `recovery::recover` must
//!   1. never panic,
//!   2. yield exactly the longest valid prefix of the log (no record lost that
//!      was fully written, no partial or stale record accepted), and
//!   3. produce the same book state as replaying that prefix through a fresh
//!      engine (determinism), whether or not a snapshot was used.
//!
//! Faults injected:
//!   * truncation of the log at every byte offset,
//!   * three corruption patterns (xor 0xFF, zero, flip bit 0) at every byte
//!     offset, each followed by "keep writing, crash again, recover again",
//!   * the same byte-level corruption of a snapshot file,
//!   * a writer process (this test binary re-executed with an env var) killed
//!     at a pseudo-random point mid-write, repeated over many cycles, with
//!     snapshots being taken while it runs.
//!
//! Run: `cargo test --release --test crash_recovery -- --nocapture`
//! (release makes the ~2,500 recovery cycles take seconds instead of a minute).
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::panic::{self, AssertUnwindSafe};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ferrox::matching::MatchingEngine;
use ferrox::order::{Order, Side};
use ferrox::protocol::EngineCommand;
use ferrox::recovery::recover_with_wal_size;
use ferrox::snapshot::Snapshot;
use ferrox::wal::Wal;

const ARENA: u32 = 8192;
const NEW_ORDER_RECORD: u64 = 48; // align_up(8 + 40)
const CANCEL_RECORD: u64 = 24; // align_up(8 + 16)
/// 1 MiB mappings instead of the 64 MiB default: ~3,300 logs are opened per run.
const WAL_SIZE: u64 = 1 << 20;
/// The child snapshots this often. Chosen so that on this hardware a kill is
/// about as likely to land inside `Snapshot::save` as between two appends.
const SNAPSHOT_EVERY: u64 = 500;

fn recover(data: &Path) -> Result<(MatchingEngine, Wal), ferrox::recovery::RecoveryError> {
    recover_with_wal_size(data, ARENA, Some(WAL_SIZE))
}

/// Deterministic command stream: a pure function of `(seed, index)`, so a
/// recovered process can regenerate exactly the commands a killed one wrote.
/// Order ids are `index + 1`. Timestamps are 0 because the WAL does not
/// persist them. Prices span 11 ticks on both sides so fills, partial fills,
/// level creation/removal and self-trade prevention all happen.
fn command(seed: u64, i: u64) -> EngineCommand {
    let mut z = seed ^ i.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    if i > 0 && z % 4 == 0 {
        EngineCommand::CancelOrder {
            order_id: 1 + (z >> 8) % i,
        }
    } else {
        let side = if (z >> 2) & 1 == 0 {
            Side::Bid
        } else {
            Side::Ask
        };
        let price = 100 + ((z >> 3) % 11) as i64;
        let qty = 1 + (z >> 16) % 20;
        let trader = (z >> 24) % 4;
        EngineCommand::NewOrder(Order::new(i + 1, trader, side, price, qty, 0).unwrap())
    }
}

fn record_size(cmd: &EngineCommand) -> u64 {
    match cmd {
        EngineCommand::NewOrder(_) => NEW_ORDER_RECORD,
        EngineCommand::CancelOrder { .. } => CANCEL_RECORD,
    }
}

fn apply(engine: &mut MatchingEngine, cmd: EngineCommand) {
    match cmd {
        EngineCommand::NewOrder(o) => {
            let _ = engine.add_order(o);
        }
        EngineCommand::CancelOrder { order_id } => {
            let _ = engine.cancel_order(order_id);
        }
    }
}

fn replay(seed: u64, n: u64) -> MatchingEngine {
    let mut engine = MatchingEngine::with_capacity(ARENA);
    for i in 0..n {
        apply(&mut engine, command(seed, i));
    }
    engine
}

type BookState = (Vec<Order>, Option<i64>, Option<i64>, usize);

fn state(engine: &MatchingEngine) -> BookState {
    let book = engine.book();
    (
        book.all_resting_orders(),
        book.best_bid(),
        book.best_ask(),
        book.order_count(),
    )
}

/// Writes `n` commands to `<data>/wal.bin`, returns the valid log bytes and
/// the end offset of every record.
fn build_wal(data: &Path, seed: u64, n: u64) -> (Vec<u8>, Vec<u64>) {
    fs::create_dir_all(data).unwrap();
    let mut wal = Wal::open(data.join("wal.bin")).unwrap();
    let mut ends = Vec::with_capacity(n as usize);
    let mut pos = 0;
    for i in 0..n {
        let cmd = command(seed, i);
        wal.append(&cmd).unwrap();
        pos += record_size(&cmd);
        ends.push(pos);
    }
    assert_eq!(wal.write_pos(), pos);
    drop(wal);
    let mut bytes = fs::read(data.join("wal.bin")).unwrap();
    bytes.truncate(pos as usize);
    (bytes, ends)
}

/// Recover, check count and state against the reference replay, then keep
/// writing three more commands, recover again and check again. Returns a
/// description of the first failure, if any; panics inside are caught so one
/// run reports every failing offset.
fn check_recovery(data: &Path, seed: u64, n: u64, expected_k: u64, label: &str) -> Option<String> {
    let result = panic::catch_unwind(AssertUnwindSafe(|| {
        let (engine, mut wal) = match recover(data) {
            Ok(x) => x,
            Err(e) => return Some(format!("{label}: recover failed: {e}")),
        };
        if wal.record_count() != expected_k {
            return Some(format!(
                "{label}: recovered {} records, expected {expected_k}",
                wal.record_count()
            ));
        }
        if state(&engine) != state(&replay(seed, expected_k)) {
            return Some(format!(
                "{label}: book state differs from replay of {expected_k} records"
            ));
        }

        // The survivor keeps running: append three more, exit cleanly, recover.
        let mut engine = engine;
        for j in 0..3 {
            let cmd = command(seed, n + j);
            wal.append(&cmd).unwrap();
            apply(&mut engine, cmd);
        }
        let expected_after = state(&engine);
        drop(wal);

        let (engine2, wal2) = match recover(data) {
            Ok(x) => x,
            Err(e) => return Some(format!("{label}: second recover failed: {e}")),
        };
        if wal2.record_count() != expected_k + 3 {
            return Some(format!(
                "{label}: after appending 3, recovered {} records, expected {} (stale records resurrected or new ones lost)",
                wal2.record_count(),
                expected_k + 3
            ));
        }
        if state(&engine2) != expected_after {
            return Some(format!(
                "{label}: after appending 3, book state differs from the writer state"
            ));
        }
        None
    }));
    match result {
        Ok(r) => r,
        Err(e) => {
            let msg = e
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_default();
            Some(format!("{label}: PANIC: {msg}"))
        }
    }
}

fn report(name: &str, cases: usize, failures: &[String]) {
    println!(
        "{name}: {cases} cases, {} failures{}",
        failures.len(),
        if failures.is_empty() { "" } else { ":" }
    );
    for f in failures.iter().take(40) {
        println!("  {f}");
    }
    if failures.len() > 40 {
        println!("  ... {} more", failures.len() - 40);
    }
    assert!(failures.is_empty(), "{name}: {} failures", failures.len());
}

#[test]
fn wal_truncated_at_every_byte_offset_recovers_the_longest_valid_prefix() {
    let seed = 0x5EED_0001;
    let n = 16;
    let base = tempfile::tempdir().unwrap();
    let (full, ends) = build_wal(&base.path().join("data"), seed, n);

    let mut failures = Vec::new();
    let mut indistinguishable = 0;
    for len in 0..=full.len() {
        let case = tempfile::tempdir().unwrap();
        let data = case.path().join("data");
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join("wal.bin"), &full[..len]).unwrap();
        // The WAL zero-extends the file on open, so a record whose missing
        // tail was all zero bytes (e.g. the high bytes of a small quantity)
        // is byte-for-byte identical to the original and legitimately
        // recovers. The oracle is therefore "leading records whose bytes are
        // unchanged after zero extension", not "records fully inside `len`".
        let mut zero_extended = full[..len].to_vec();
        zero_extended.resize(full.len(), 0);
        let mut expected_k = 0u64;
        let mut start = 0usize;
        for &end in &ends {
            let end = end as usize;
            if zero_extended[start..end] != full[start..end] {
                break;
            }
            expected_k += 1;
            start = end;
        }
        let complete = ends.iter().filter(|&&e| e as usize <= len).count() as u64;
        if expected_k > complete {
            indistinguishable += 1;
        }
        if let Some(f) = check_recovery(&data, seed, n, expected_k, &format!("truncate@{len}")) {
            failures.push(f);
        }
    }
    println!(
        "wal truncation: {indistinguishable} offsets cut only trailing zero bytes of a record (indistinguishable from intact, correctly recovered)"
    );
    report("wal truncation", full.len() + 1, &failures);
}

#[test]
fn wal_corrupted_at_every_byte_offset_recovers_the_prefix_before_the_damage() {
    let seed = 0x5EED_0002;
    let n = 16;
    let base = tempfile::tempdir().unwrap();
    let (full, ends) = build_wal(&base.path().join("data"), seed, n);

    let mutations: [(&str, fn(u8) -> u8); 3] = [
        ("xor_ff", |b| b ^ 0xFF),
        ("zero", |_| 0),
        ("flip_bit0", |b| b ^ 0x01),
    ];

    let mut cases = 0;
    let mut failures = Vec::new();
    for offset in 0..full.len() {
        // Records strictly before the one containing `offset` must survive.
        let expected_k = ends.iter().filter(|&&e| e as usize <= offset).count() as u64;
        for (name, mutate) in &mutations {
            let mut bytes = full.clone();
            let new = mutate(bytes[offset]);
            if new == bytes[offset] {
                continue;
            }
            bytes[offset] = new;
            cases += 1;

            let case = tempfile::tempdir().unwrap();
            let data = case.path().join("data");
            fs::create_dir_all(&data).unwrap();
            fs::write(data.join("wal.bin"), &bytes).unwrap();
            if let Some(f) = check_recovery(
                &data,
                seed,
                n,
                expected_k,
                &format!("corrupt@{offset}/{name}"),
            ) {
                failures.push(f);
            }
        }
    }
    report("wal corruption", cases, &failures);
}

#[test]
fn snapshot_corrupted_at_every_byte_offset_never_yields_a_divergent_state() {
    let seed = 0x5EED_0003;
    let n = 40;
    let k = 25; // snapshot taken after record k

    let base = tempfile::tempdir().unwrap();
    let data = base.path().join("data");
    let (wal_bytes, _) = build_wal(&data, seed, n);
    let snap_dir = data.join("snapshots");
    let snap_path = Snapshot::capture(&replay(seed, k), k)
        .save(&snap_dir)
        .unwrap();
    let snap_bytes = fs::read(&snap_path).unwrap();
    let snap_name = snap_path.file_name().unwrap().to_owned();

    // Sanity: the intact snapshot + WAL tail equals the full replay.
    let truth = state(&replay(seed, n));
    let (engine, wal) = recover(&data).unwrap();
    assert_eq!(wal.record_count(), n);
    assert_eq!(state(&engine), truth);
    drop((engine, wal));

    let mutations: [(&str, fn(u8) -> u8); 3] = [
        ("xor_ff", |b| b ^ 0xFF),
        ("zero", |_| 0),
        ("flip_bit0", |b| b ^ 0x01),
    ];

    let mut cases = 0;
    let mut failures = Vec::new();
    for offset in 0..snap_bytes.len() {
        for (name, mutate) in &mutations {
            let mut bytes = snap_bytes.clone();
            let new = mutate(bytes[offset]);
            if new == bytes[offset] {
                continue;
            }
            bytes[offset] = new;
            cases += 1;

            let case = tempfile::tempdir().unwrap();
            let cdata = case.path().join("data");
            fs::create_dir_all(cdata.join("snapshots")).unwrap();
            fs::write(cdata.join("wal.bin"), &wal_bytes).unwrap();
            fs::write(cdata.join("snapshots").join(&snap_name), &bytes).unwrap();

            let label = format!("snapshot corrupt@{offset}/{name}");
            let outcome = panic::catch_unwind(AssertUnwindSafe(|| match recover(&cdata) {
                Ok((engine, wal)) => {
                    if wal.record_count() != n {
                        Some(format!(
                            "{label}: {} WAL records visible, expected {n}",
                            wal.record_count()
                        ))
                    } else if state(&engine) != truth {
                        Some(format!(
                            "{label}: recovered state diverges from the full replay (snapshot accepted but wrong)"
                        ))
                    } else {
                        None
                    }
                }
                Err(e) => Some(format!("{label}: recover failed: {e}")),
            }));
            match outcome {
                Ok(None) => {}
                Ok(Some(f)) => failures.push(f),
                Err(_) => failures.push(format!("{label}: PANIC")),
            }
        }
    }
    println!("snapshot file: {} bytes", snap_bytes.len());
    report("snapshot corruption", cases, &failures);
}

// ---------------------------------------------------------------------------
// Process-kill test. The child is this same test binary, filtered to
// `crash_child_main`, with FERROX_CRASH_DIR set.
// ---------------------------------------------------------------------------

/// Not a test of its own: when `FERROX_CRASH_DIR` is set this is the writer
/// process that `killed_writer_recovers_to_a_consistent_prefix` kills.
#[test]
fn crash_child_main() {
    let Some(dir) = std::env::var_os("FERROX_CRASH_DIR") else {
        return;
    };
    let seed: u64 = std::env::var("FERROX_CRASH_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let data = Path::new(&dir);
    let snap_dir = data.join("snapshots");

    // Resume from whatever the previous (killed) incarnation left behind.
    let (mut engine, mut wal) = recover(data).expect("child recover");
    let mut i = wal.record_count();

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    writeln!(out, "resumed {i}").unwrap();
    out.flush().unwrap();

    // Runs until the parent kills it: a fixed cap made the child finish early on a
    // quiet machine (cycle 8 of 12 found nothing left to write and timed out).
    loop {
        let cmd = command(seed, i);
        wal.append(&cmd).expect("append");
        apply(&mut engine, cmd);
        i += 1;
        // Acknowledge only after the append returned: the parent requires
        // recovered_count >= last ack.
        writeln!(out, "acked {i}").unwrap();
        out.flush().unwrap();
        if i % SNAPSHOT_EVERY == 0 {
            Snapshot::capture(&engine, i)
                .save(&snap_dir)
                .expect("snapshot");
        }
    }
}

#[test]
fn killed_writer_recovers_to_a_consistent_prefix() {
    if std::env::var_os("FERROX_CRASH_DIR").is_some() {
        return; // we are the child; only crash_child_main runs there
    }
    let seed = 0xC0FF_EE42u64;
    let cycles = 12;
    let base = tempfile::tempdir().unwrap();
    let data = base.path().join("data");
    fs::create_dir_all(&data).unwrap();
    let exe = std::env::current_exe().unwrap();

    let mut last_count = 0;
    let mut tmp_leftovers = 0;
    let mut snapshot_cycles = 0;
    let mut kills_during_save = 0;
    for cycle in 0..cycles {
        let mut child = Command::new(&exe)
            .args([
                "crash_child_main",
                "--exact",
                "--nocapture",
                "--test-threads=1",
            ])
            .env("FERROX_CRASH_DIR", &data)
            .env("FERROX_CRASH_SEED", seed.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn child");

        // Read the child's progress live so the kill lands while it is
        // writing, not during process start-up.
        let stdout = child.stdout.take().unwrap();
        let lines: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let reader = {
            let lines = Arc::clone(&lines);
            std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    lines.lock().unwrap().push(line);
                }
            })
        };
        let started = std::time::Instant::now();
        while !lines
            .lock()
            .unwrap()
            .iter()
            .any(|l| l.starts_with("acked "))
        {
            assert!(
                started.elapsed() < Duration::from_secs(20),
                "cycle {cycle}: child never acknowledged a record"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
        // Vary the kill point within the write stream.
        std::thread::sleep(Duration::from_millis(5 + (cycle as u64 * 13) % 60));
        child.kill().expect("kill");
        child.wait().expect("wait");
        reader.join().unwrap();
        let text = lines.lock().unwrap();
        let last_ack: Option<u64> = text
            .iter()
            .filter_map(|l| l.strip_prefix("acked "))
            .filter_map(|v| v.trim().parse().ok())
            .last();
        let resumed: Option<u64> = text
            .iter()
            .filter_map(|l| l.split("resumed ").nth(1))
            .filter_map(|v| v.trim().parse().ok())
            .next();
        assert_eq!(
            resumed,
            Some(last_count),
            "cycle {cycle}: child resumed from the wrong record count"
        );

        if fs::read_dir(data.join("snapshots"))
            .map(|d| {
                d.flatten()
                    .any(|e| e.path().extension().is_some_and(|x| x == "tmp"))
            })
            .unwrap_or(false)
        {
            tmp_leftovers += 1;
        }

        let (engine, wal) = recover(&data).expect("recover after kill");
        let count = wal.record_count();
        if let Some(ack) = last_ack {
            assert!(
                count >= ack && count <= ack + 1,
                "cycle {cycle}: recovered {count} records but the writer acknowledged {ack}"
            );
        }
        assert!(
            count >= last_count,
            "cycle {cycle}: recovered {count} < {last_count} from the previous cycle"
        );
        assert_eq!(
            state(&engine),
            state(&replay(seed, count)),
            "cycle {cycle}: recovered book differs from a fresh replay of {count} commands"
        );

        // The snapshot path must agree with a WAL-only replay of the same log.
        let mut wal_only = MatchingEngine::with_capacity(ARENA);
        for rec in wal.iter_from(0) {
            let (_, cmd) = rec.expect("valid record");
            apply(&mut wal_only, cmd);
        }
        assert_eq!(
            state(&engine),
            state(&wal_only),
            "cycle {cycle}: snapshot+tail != WAL-only replay"
        );
        if Snapshot::load_latest(&data.join("snapshots"))
            .unwrap()
            .is_some()
        {
            snapshot_cycles += 1;
        }
        let in_save = count % SNAPSHOT_EVERY == 0;
        if in_save {
            kills_during_save += 1;
        }
        println!(
            "cycle {cycle}: last ack {last_ack:?}, recovered {count} records, {} resting orders, killed {}",
            engine.book().order_count(),
            if in_save {
                "during snapshot save"
            } else {
                "between appends"
            }
        );
        last_count = count;
    }
    println!(
        "process-kill: {cycles} cycles, {last_count} records total, {kills_during_save} kills during a snapshot save, {} between appends, {snapshot_cycles} cycles recovered through a snapshot, {tmp_leftovers} cycles saw a leftover .tmp snapshot",
        cycles - kills_during_save
    );
    assert!(last_count > 0, "the writer never made progress");
}
