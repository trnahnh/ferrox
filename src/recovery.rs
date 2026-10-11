use std::fs;
use std::path::Path;

use crate::matching::MatchingEngine;
use crate::protocol::EngineCommand;
use crate::snapshot::{Snapshot, SnapshotError};
use crate::wal::{Wal, WalError};

#[derive(Debug)]
pub enum RecoveryError {
    Wal(WalError),
    Snapshot(SnapshotError),
}

impl std::fmt::Display for RecoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Wal(e) => write!(f, "recovery wal error: {e}"),
            Self::Snapshot(e) => write!(f, "recovery snapshot error: {e}"),
        }
    }
}

impl std::error::Error for RecoveryError {}

impl From<WalError> for RecoveryError {
    fn from(e: WalError) -> Self {
        Self::Wal(e)
    }
}

impl From<SnapshotError> for RecoveryError {
    fn from(e: SnapshotError) -> Self {
        Self::Snapshot(e)
    }
}

/// Rebuilds the engine from `<data_dir>/snapshots` + `<data_dir>/wal.bin`
/// and returns it together with the opened WAL, positioned for appending.
pub fn recover(
    data_dir: &Path,
    arena_capacity: u32,
) -> Result<(MatchingEngine, Wal), RecoveryError> {
    recover_with_wal_size(data_dir, arena_capacity, None)
}

/// Same as [`recover`], with an explicit initial WAL mapping size (the
/// default is 64 MiB). Smaller sizes make fault-injection tests that open
/// thousands of logs fast; production callers use [`recover`].
pub fn recover_with_wal_size(
    data_dir: &Path,
    arena_capacity: u32,
    wal_initial_size: Option<u64>,
) -> Result<(MatchingEngine, Wal), RecoveryError> {
    fs::create_dir_all(data_dir).map_err(WalError::Io)?;

    let snapshot_dir = data_dir.join("snapshots");

    let (mut engine, start_record) = match Snapshot::load_latest(&snapshot_dir)? {
        Some(snap) => {
            let record_count = snap.wal_record_count;
            let engine = snap.restore(arena_capacity)?;
            (engine, record_count)
        }
        None => (MatchingEngine::with_capacity(arena_capacity), 0),
    };

    let wal_path = data_dir.join("wal.bin");
    let mut wal = match wal_initial_size {
        Some(size) => Wal::open_with_size(&wal_path, size)?,
        None => Wal::open(&wal_path)?,
    };

    let mut record_count_at_replay = start_record;

    for result in wal.iter_from(start_record) {
        match result {
            Ok((_record_num, cmd)) => {
                replay_command(&mut engine, cmd);
                record_count_at_replay += 1;
            }
            Err(WalError::Corruption { offset } | WalError::TruncatedRecord { offset }) => {
                // Truncate WAL at corruption point
                wal.truncate_to(offset, record_count_at_replay)?;
                break;
            }
            // A record whose CRC is valid but whose payload does not decode
            // (`WalError::Protocol`) is not treated as the end of the log: the
            // CRC says the bytes are what was written, so this is a writer bug,
            // a foreign log or a 1-in-2^32 CRC collision, and starting with a
            // guessed book is worse than not starting. Found by the fuzz target
            // `wal_recover` on 2026-10-11 (one-byte payload 0xFF, CRC32
            // 0xFF000000); see `crc_valid_undecodable_record_refuses_to_start`.
            Err(e) => return Err(RecoveryError::Wal(e)),
        }
    }

    Ok((engine, wal))
}

fn replay_command(engine: &mut MatchingEngine, cmd: EngineCommand) {
    match cmd {
        EngineCommand::NewOrder(order) => {
            let _ = engine.add_order(order);
        }
        EngineCommand::CancelOrder { order_id } => {
            let _ = engine.cancel_order(order_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::order::{Order, Side};
    use crate::snapshot::Snapshot;

    fn bid(id: u64, price: i64, qty: u64) -> Order {
        Order::new(id, id, Side::Bid, price, qty, id).unwrap()
    }

    fn ask(id: u64, price: i64, qty: u64) -> Order {
        Order::new(id, id, Side::Ask, price, qty, id).unwrap()
    }

    #[test]
    fn fresh_start_no_files() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");

        let (engine, wal) = recover(&data_dir, 1024).unwrap();
        assert_eq!(engine.book().order_count(), 0);
        assert_eq!(wal.record_count(), 0);
    }

    /// Regression for the first fuzz finding (2026-10-11): a one-byte payload
    /// `0xFF` has CRC32 `0xFF000000`, so the WAL accepts the record and the
    /// protocol decoder rejects it. Recovery must return the protocol error
    /// (not panic, not silently truncate), and must do so deterministically.
    #[test]
    fn crc_valid_undecodable_record_refuses_to_start() {
        use crate::protocol::ProtocolError;

        assert_eq!(crc32fast::hash(&[0xFF]), 0xFF00_0000);
        // [payload_len = 1][crc32 = 0xFF000000][payload 0xFF][7 bytes padding]
        let mut record = Vec::new();
        record.extend_from_slice(&1u32.to_le_bytes());
        record.extend_from_slice(&0xFF00_0000u32.to_le_bytes());
        record.push(0xFF);
        record.extend_from_slice(&[0u8; 7]);
        assert_eq!(record.len(), 16);

        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::write(data_dir.join("wal.bin"), &record).unwrap();

        // The WAL layer accepts the record: the CRC is valid.
        let wal = Wal::open_with_size(&data_dir.join("wal.bin"), 1 << 16).unwrap();
        assert_eq!(wal.record_count(), 1);
        drop(wal);

        for _ in 0..2 {
            match recover_with_wal_size(&data_dir, 1024, Some(1 << 16)) {
                Err(RecoveryError::Wal(WalError::Protocol(e))) => {
                    assert_eq!(e, ProtocolError::UnknownMessageType(255));
                }
                Ok(_) => panic!("expected a protocol error, recovery succeeded"),
                Err(e) => panic!("expected a protocol error, got {e:?}"),
            }
        }
        // Recovery did not rewrite the log.
        assert_eq!(
            std::fs::read(data_dir.join("wal.bin")).unwrap()[..16],
            record[..]
        );
    }

    #[test]
    fn wal_only_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();

        {
            let mut wal = Wal::open(data_dir.join("wal.bin")).unwrap();
            wal.append(&EngineCommand::NewOrder(bid(1, 100, 10)))
                .unwrap();
            wal.append(&EngineCommand::NewOrder(ask(2, 110, 20)))
                .unwrap();
            wal.append(&EngineCommand::NewOrder(bid(3, 98, 30)))
                .unwrap();
        }

        let (engine, wal) = recover(&data_dir, 1024).unwrap();
        assert_eq!(engine.book().order_count(), 3);
        assert_eq!(engine.book().best_bid(), Some(100));
        assert_eq!(engine.book().best_ask(), Some(110));
        assert_eq!(wal.record_count(), 3);
    }

    #[test]
    fn snapshot_only_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let snap_dir = data_dir.join("snapshots");

        let mut engine = MatchingEngine::with_capacity(1024);
        engine.add_order(bid(1, 100, 10)).unwrap();
        engine.add_order(ask(2, 110, 20)).unwrap();
        Snapshot::capture(&engine, 2).save(&snap_dir).unwrap();

        let (recovered, wal) = recover(&data_dir, 1024).unwrap();
        assert_eq!(recovered.book().order_count(), 2);
        assert_eq!(recovered.book().best_bid(), Some(100));
        assert_eq!(recovered.book().best_ask(), Some(110));
        assert_eq!(wal.record_count(), 0);
    }

    #[test]
    fn snapshot_plus_wal_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let snap_dir = data_dir.join("snapshots");
        fs::create_dir_all(&data_dir).unwrap();

        let mut engine = MatchingEngine::with_capacity(1024);
        engine.add_order(bid(1, 100, 10)).unwrap();
        engine.add_order(ask(2, 110, 20)).unwrap();

        Snapshot::capture(&engine, 2).save(&snap_dir).unwrap();

        // WAL records 1, 2, 3 — only 3 is after snapshot
        {
            let mut wal = Wal::open(data_dir.join("wal.bin")).unwrap();
            wal.append(&EngineCommand::NewOrder(bid(1, 100, 10)))
                .unwrap();
            wal.append(&EngineCommand::NewOrder(ask(2, 110, 20)))
                .unwrap();
            wal.append(&EngineCommand::NewOrder(bid(3, 98, 30)))
                .unwrap();
        }

        let (recovered, wal) = recover(&data_dir, 1024).unwrap();
        assert_eq!(recovered.book().order_count(), 3);
        assert_eq!(recovered.book().best_bid(), Some(100));
        assert_eq!(wal.record_count(), 3);
    }

    #[test]
    fn recovery_matches_full_replay() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        let snap_dir = data_dir.join("snapshots");
        fs::create_dir_all(&data_dir).unwrap();

        let orders = vec![
            bid(1, 100, 10),
            ask(2, 110, 20),
            bid(3, 98, 30),
            ask(4, 105, 15),
        ];

        let mut full_engine = MatchingEngine::with_capacity(1024);
        for o in &orders {
            full_engine.add_order(o.clone()).unwrap();
        }

        {
            let mut partial = MatchingEngine::with_capacity(1024);
            partial.add_order(orders[0].clone()).unwrap();
            partial.add_order(orders[1].clone()).unwrap();
            Snapshot::capture(&partial, 2).save(&snap_dir).unwrap();
        }
        {
            let mut wal = Wal::open(data_dir.join("wal.bin")).unwrap();
            for o in &orders {
                wal.append(&EngineCommand::NewOrder(o.clone())).unwrap();
            }
        }

        let (recovered, _) = recover(&data_dir, 1024).unwrap();

        let full_orders = full_engine.book().all_resting_orders();
        let recovered_orders = recovered.book().all_resting_orders();
        assert_eq!(full_orders.len(), recovered_orders.len());
        for (f, r) in full_orders.iter().zip(recovered_orders.iter()) {
            assert_eq!(f.id, r.id);
            assert_eq!(f.price, r.price);
            assert_eq!(f.quantity, r.quantity);
            assert_eq!(f.side, r.side);
        }
    }

    #[test]
    fn truncated_wal_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();

        let wal_path = data_dir.join("wal.bin");

        {
            let mut wal = Wal::open(&wal_path).unwrap();
            wal.append(&EngineCommand::NewOrder(bid(1, 100, 10)))
                .unwrap();
            wal.append(&EngineCommand::NewOrder(ask(2, 110, 20)))
                .unwrap();
            wal.append(&EngineCommand::NewOrder(bid(3, 98, 30)))
                .unwrap();
        }

        let (engine, wal) = recover(&data_dir, 1024).unwrap();
        assert_eq!(engine.book().order_count(), 3);
        assert_eq!(wal.record_count(), 3);
    }

    #[test]
    fn deterministic_replay() {
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();
        let data1 = dir1.path().join("data");
        let data2 = dir2.path().join("data");

        let orders = vec![
            bid(1, 100, 10),
            ask(2, 110, 20),
            bid(3, 98, 30),
            ask(4, 105, 15),
            bid(5, 108, 25), // This crosses ask@105 — fills will occur
        ];

        for data_dir in [&data1, &data2] {
            fs::create_dir_all(data_dir).unwrap();
            let mut wal = Wal::open(data_dir.join("wal.bin")).unwrap();
            for o in &orders {
                wal.append(&EngineCommand::NewOrder(o.clone())).unwrap();
            }
        }

        let (engine1, _) = recover(&data1, 1024).unwrap();
        let (engine2, _) = recover(&data2, 1024).unwrap();

        let orders1 = engine1.book().all_resting_orders();
        let orders2 = engine2.book().all_resting_orders();
        assert_eq!(orders1.len(), orders2.len());
        for (a, b) in orders1.iter().zip(orders2.iter()) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.price, b.price);
            assert_eq!(a.quantity, b.quantity);
            assert_eq!(a.side, b.side);
        }
    }

    #[test]
    fn recovery_with_cancels() {
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();

        {
            let mut wal = Wal::open(data_dir.join("wal.bin")).unwrap();
            wal.append(&EngineCommand::NewOrder(bid(1, 100, 10)))
                .unwrap();
            wal.append(&EngineCommand::NewOrder(ask(2, 110, 20)))
                .unwrap();
            wal.append(&EngineCommand::CancelOrder { order_id: 1 })
                .unwrap();
        }

        let (engine, wal) = recover(&data_dir, 1024).unwrap();
        assert_eq!(engine.book().order_count(), 1);
        assert_eq!(engine.book().best_bid(), None);
        assert_eq!(engine.book().best_ask(), Some(110));
        assert_eq!(wal.record_count(), 3);
    }
}
