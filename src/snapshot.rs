use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::matching::MatchingEngine;
use crate::order::Order;

#[derive(Debug)]
pub enum SnapshotError {
    Io(io::Error),
    Serialize(String),
    Deserialize(String),
    ChecksumMismatch { expected: u32, actual: u32 },
    Restore(String),
}

impl std::fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "snapshot io error: {e}"),
            Self::Serialize(e) => write!(f, "snapshot serialize error: {e}"),
            Self::Deserialize(e) => write!(f, "snapshot deserialize error: {e}"),
            Self::ChecksumMismatch { expected, actual } => write!(
                f,
                "snapshot checksum mismatch: expected {expected:#010x}, got {actual:#010x}"
            ),
            Self::Restore(e) => write!(f, "snapshot restore error: {e}"),
        }
    }
}

impl std::error::Error for SnapshotError {}

impl From<io::Error> for SnapshotError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub wal_record_count: u64,
    pub(crate) orders: Vec<Order>,
    pub(crate) best_bid: Option<i64>,
    pub(crate) best_ask: Option<i64>,
    /// CRC32 over the bincode serialization of every other field
    /// (`wal_record_count`, `best_bid`, `best_ask`, `orders`). Until
    /// 2026-10-11 only `orders` was covered, so a corrupted
    /// `wal_record_count` was accepted and replay started from the wrong
    /// record (found by tests/crash_recovery.rs).
    pub(crate) checksum: u32,
}

impl Snapshot {
    pub fn capture(engine: &MatchingEngine, wal_record_count: u64) -> Self {
        let orders = engine.book().all_resting_orders();
        let best_bid = engine.book().best_bid();
        let best_ask = engine.book().best_ask();
        let checksum = Self::compute_checksum(wal_record_count, best_bid, best_ask, &orders);

        Self {
            wal_record_count,
            orders,
            best_bid,
            best_ask,
            checksum,
        }
    }

    /// Atomic save: write to temp file, then rename.
    pub fn save(&self, dir: &Path) -> Result<PathBuf, SnapshotError> {
        fs::create_dir_all(dir)?;

        let filename = format!("snapshot_{:010}.bin", self.wal_record_count);
        let final_path = dir.join(&filename);
        let tmp_path = dir.join(format!("{filename}.tmp"));

        let data = bincode::serialize(self).map_err(|e| SnapshotError::Serialize(e.to_string()))?;

        fs::write(&tmp_path, &data)?;
        fs::rename(&tmp_path, &final_path)?;

        Ok(final_path)
    }

    /// Returns `Ok(None)` if the directory is empty or doesn't exist.
    pub fn load_latest(dir: &Path) -> Result<Option<Self>, SnapshotError> {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(SnapshotError::Io(e)),
        };

        let mut snapshot_files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("snapshot_") && n.ends_with(".bin"))
            })
            .collect();

        // Sort lexicographically — highest (most recent) last.
        snapshot_files.sort();

        while let Some(path) = snapshot_files.pop() {
            match Self::load_from_file(&path) {
                Ok(snap) => {
                    if snap.verify_checksum().is_ok() {
                        return Ok(Some(snap));
                    }
                    // Checksum failed — try older snapshot.
                }
                Err(_) => {
                    // Corrupt file — try older snapshot.
                }
            }
        }

        Ok(None)
    }

    pub fn restore(&self, arena_capacity: u32) -> Result<MatchingEngine, SnapshotError> {
        MatchingEngine::restore_from_orders(&self.orders, arena_capacity)
            .map_err(|e| SnapshotError::Restore(format!("{e:?}")))
    }

    pub fn verify_checksum(&self) -> Result<(), SnapshotError> {
        let actual = Self::compute_checksum(
            self.wal_record_count,
            self.best_bid,
            self.best_ask,
            &self.orders,
        );
        if self.checksum == actual {
            Ok(())
        } else {
            Err(SnapshotError::ChecksumMismatch {
                expected: self.checksum,
                actual,
            })
        }
    }

    fn compute_checksum(
        wal_record_count: u64,
        best_bid: Option<i64>,
        best_ask: Option<i64>,
        orders: &[Order],
    ) -> u32 {
        let bytes = bincode::serialize(&(wal_record_count, best_bid, best_ask, orders))
            .expect("serializing snapshot fields for checksum should not fail");
        crc32fast::hash(&bytes)
    }

    fn load_from_file(path: &Path) -> Result<Self, SnapshotError> {
        let data = fs::read(path)?;
        bincode::deserialize(&data).map_err(|e| SnapshotError::Deserialize(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matching::MatchingEngine;
    use crate::order::{Order, Side};

    fn bid(id: u64, price: i64, qty: u64) -> Order {
        Order::new(id, 1, Side::Bid, price, qty, id).unwrap()
    }

    fn ask(id: u64, price: i64, qty: u64) -> Order {
        Order::new(id, 1, Side::Ask, price, qty, id).unwrap()
    }

    fn engine_with_orders(orders: &[Order]) -> MatchingEngine {
        let mut engine = MatchingEngine::with_capacity(1024);
        for o in orders {
            engine.add_order(o.clone()).unwrap();
        }
        engine
    }

    #[test]
    fn capture_empty_book() {
        let engine = MatchingEngine::with_capacity(64);
        let snap = Snapshot::capture(&engine, 0);

        assert_eq!(snap.wal_record_count, 0);
        assert!(snap.orders.is_empty());
        assert_eq!(snap.best_bid, None);
        assert_eq!(snap.best_ask, None);
        snap.verify_checksum().unwrap();
    }

    #[test]
    fn capture_with_orders() {
        let engine = engine_with_orders(&[bid(1, 100, 10), ask(2, 110, 20)]);
        let snap = Snapshot::capture(&engine, 5);

        assert_eq!(snap.wal_record_count, 5);
        assert_eq!(snap.orders.len(), 2);
        assert_eq!(snap.best_bid, Some(100));
        assert_eq!(snap.best_ask, Some(110));
        snap.verify_checksum().unwrap();
    }

    #[test]
    fn save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let engine = engine_with_orders(&[bid(1, 100, 10), ask(2, 110, 20), bid(3, 98, 30)]);
        let snap = Snapshot::capture(&engine, 42);

        let path = snap.save(dir.path()).unwrap();
        assert!(path.exists());
        assert!(
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .contains("0000000042")
        );

        let loaded = Snapshot::load_latest(dir.path()).unwrap().unwrap();
        assert_eq!(loaded.wal_record_count, 42);
        assert_eq!(loaded.orders.len(), 3);
        assert_eq!(loaded.best_bid, Some(100));
        assert_eq!(loaded.best_ask, Some(110));
        loaded.verify_checksum().unwrap();
    }

    #[test]
    fn checksum_detects_corruption() {
        let engine = engine_with_orders(&[bid(1, 100, 10)]);
        let mut snap = Snapshot::capture(&engine, 1);

        snap.verify_checksum().unwrap();

        snap.orders[0].quantity = 999;
        assert!(snap.verify_checksum().is_err());
    }

    #[test]
    fn checksum_covers_wal_record_count() {
        let engine = engine_with_orders(&[bid(1, 100, 10)]);
        let mut snap = Snapshot::capture(&engine, 7);
        snap.verify_checksum().unwrap();

        // A wrong record count makes recovery replay from the wrong place,
        // so it must be rejected like any other corruption.
        snap.wal_record_count = 6;
        assert!(snap.verify_checksum().is_err());
    }

    #[test]
    fn restore_produces_identical_book() {
        let orders = vec![bid(1, 100, 10), ask(2, 110, 20), bid(3, 98, 30)];
        let engine = engine_with_orders(&orders);
        let snap = Snapshot::capture(&engine, 10);

        let restored = snap.restore(1024).unwrap();
        assert_eq!(restored.book().order_count(), 3);
        assert_eq!(restored.book().best_bid(), Some(100));
        assert_eq!(restored.book().best_ask(), Some(110));

        let restored_orders = restored.book().all_resting_orders();
        assert_eq!(restored_orders.len(), snap.orders.len());
        for (orig, rest) in snap.orders.iter().zip(restored_orders.iter()) {
            assert_eq!(orig.id, rest.id);
            assert_eq!(orig.price, rest.price);
            assert_eq!(orig.quantity, rest.quantity);
            assert_eq!(orig.side, rest.side);
        }
    }

    #[test]
    fn restore_then_match() {
        let orders = vec![ask(1, 100, 10)];
        let engine = engine_with_orders(&orders);
        let snap = Snapshot::capture(&engine, 1);

        let mut restored = snap.restore(1024).unwrap();
        let result = restored
            .add_order(Order::new(2, 2, Side::Bid, 100, 10, 2).unwrap())
            .unwrap();
        assert_eq!(result.fills.len(), 1);
        assert_eq!(result.fills[0].maker_order_id, 1);
    }

    #[test]
    fn load_latest_picks_newest() {
        let dir = tempfile::tempdir().unwrap();

        let engine1 = engine_with_orders(&[bid(1, 100, 10)]);
        Snapshot::capture(&engine1, 10).save(dir.path()).unwrap();

        let engine2 = engine_with_orders(&[bid(1, 100, 10), ask(2, 110, 20)]);
        Snapshot::capture(&engine2, 20).save(dir.path()).unwrap();

        let loaded = Snapshot::load_latest(dir.path()).unwrap().unwrap();
        assert_eq!(loaded.wal_record_count, 20);
        assert_eq!(loaded.orders.len(), 2);
    }

    #[test]
    fn load_latest_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = Snapshot::load_latest(dir.path()).unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn load_latest_nonexistent_dir() {
        let loaded = Snapshot::load_latest(Path::new("/nonexistent/snapshot/dir")).unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn load_latest_skips_corrupt_file() {
        let dir = tempfile::tempdir().unwrap();

        let engine = engine_with_orders(&[bid(1, 100, 10)]);
        Snapshot::capture(&engine, 10).save(dir.path()).unwrap();

        let corrupt_path = dir.path().join("snapshot_0000000020.bin");
        fs::write(&corrupt_path, b"garbage data").unwrap();

        let loaded = Snapshot::load_latest(dir.path()).unwrap().unwrap();
        assert_eq!(loaded.wal_record_count, 10);
    }
}
