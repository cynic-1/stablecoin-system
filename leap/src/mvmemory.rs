use crossbeam::utils::CachePadded;
use dashmap::DashMap;
use std::{
    collections::btree_map::BTreeMap,
    hash::Hash,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
};

pub type TxnIndex = usize;
pub type Incarnation = usize;
pub type Version = (TxnIndex, Incarnation);

const FLAG_DONE: usize = 0;
const FLAG_ESTIMATE: usize = 1;

/// Entry stored in the multi-version data structure for each write.
struct WriteCell<V> {
    flag: AtomicUsize,
    incarnation: Incarnation,
    data: Arc<V>,
}

impl<V> WriteCell<V> {
    fn new(flag: usize, incarnation: Incarnation, data: V) -> Self {
        WriteCell {
            flag: AtomicUsize::new(flag),
            incarnation,
            data: Arc::new(data),
        }
    }

    fn flag(&self) -> usize {
        self.flag.load(Ordering::SeqCst)
    }

    fn mark_estimate(&self) {
        self.flag.store(FLAG_ESTIMATE, Ordering::SeqCst);
    }
}

/// Multi-version hashmap: maps each key to a BTreeMap of (txn_idx → WriteCell).
/// DashMap provides per-key concurrent access.
pub struct MVHashMap<K, V> {
    data: DashMap<K, BTreeMap<TxnIndex, CachePadded<WriteCell<V>>>>,
}

impl<K: Hash + Clone + Eq, V> MVHashMap<K, V> {
    pub fn new() -> Self {
        MVHashMap {
            data: DashMap::new(),
        }
    }

    /// Write a versioned value at a key.
    pub fn write(&self, key: &K, version: Version, data: V) {
        let (txn_idx, incarnation) = version;
        let mut map = self.data.entry(key.clone()).or_insert_with(BTreeMap::new);
        let prev = map.insert(
            txn_idx,
            CachePadded::new(WriteCell::new(FLAG_DONE, incarnation, data)),
        );
        // In normal Block-STM operation, each new incarnation is strictly greater.
        // A stale write (from a concurrent old incarnation) should not occur because
        // resume() only transitions Executing→ReadyToExecute (bailed txns that
        // never wrote). Debug-assert for safety.
        debug_assert!(prev
            .map(|cell| cell.incarnation < incarnation)
            .unwrap_or(true));
    }

    /// Mark an entry as an estimated write (blocks future readers).
    pub fn mark_estimate(&self, key: &K, txn_idx: TxnIndex) {
        let map = self.data.get(key).expect("Path must exist");
        map.get(&txn_idx)
            .expect("Entry by txn must exist")
            .mark_estimate();
    }

    /// Delete an entry.
    pub fn delete(&self, key: &K, txn_idx: TxnIndex) {
        let mut map = self.data.get_mut(key).expect("Path must exist");
        map.remove(&txn_idx);
    }

    /// Read the latest version written before txn_idx.
    /// Returns Ok((version, data)) on success.
    /// Returns Err(Some(dep_txn_idx)) if a dependency (ESTIMATE) is found.
    /// Returns Err(None) if no prior write exists (read from storage).
    pub fn read(&self, key: &K, txn_idx: TxnIndex) -> Result<(Version, Arc<V>), Option<TxnIndex>> {
        match self.data.get(key) {
            Some(tree) => {
                let mut iter = tree.range(0..txn_idx);
                if let Some((idx, write_cell)) = iter.next_back() {
                    let flag = write_cell.flag();
                    if flag == FLAG_ESTIMATE {
                        Err(Some(*idx))
                    } else {
                        debug_assert!(flag == FLAG_DONE);
                        let write_version = (*idx, write_cell.incarnation);
                        Ok((write_version, write_cell.data.clone()))
                    }
                } else {
                    Err(None)
                }
            }
            None => Err(None),
        }
    }
}
