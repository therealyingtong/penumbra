use std::{pin::Pin, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use futures::Stream;
use jmt::storage::{LeafNode, Node, NodeKey, TreeReader};
use tokio::sync::mpsc;
use tracing::Span;

use crate::state::StateRead;

mod rocks_wrapper;
use rocks_wrapper::RocksDbSnapshot;

/// A snapshot of the underlying storage at a specific state version, suitable
/// for read-only access by multiple threads, e.g., RPC calls.
///
/// Snapshots are cheap to create and clone.  Internally, they're implemented as
/// a wrapper around a [RocksDB
/// snapshot](https://github.com/facebook/rocksdb/wiki/Snapshot) with a pinned
/// JMT version number for the snapshot.
#[derive(Clone)]
pub struct Snapshot(Arc<Inner>);

// We don't want to expose the `TreeReader` implementation outside of this crate.
struct Inner {
    snapshot: RocksDbSnapshot,
    version: jmt::Version,
    // Used to retrieve column family handles.
    db: Arc<rocksdb::DB>,
}

impl Snapshot {
    pub(crate) fn new(db: Arc<rocksdb::DB>, version: jmt::Version) -> Self {
        Self(Arc::new(Inner {
            snapshot: RocksDbSnapshot::new(db.clone()),
            version,
            db,
        }))
    }

    pub fn version(&self) -> jmt::Version {
        self.0.version
    }
}

#[async_trait]
impl StateRead for Snapshot {
    /// Fetch a key from the JMT column family.
    async fn get_raw(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let span = Span::current();
        let inner = self.0.clone();
        let key = key.to_string();
        tokio::task::Builder::new()
            .name("Snapshot::get_raw")
            .spawn_blocking(move || {
                span.in_scope(|| {
                    let jmt_cf = inner
                        .db
                        .cf_handle("jmt")
                        .expect("jmt column family not found");
                    inner.snapshot.get_cf(jmt_cf, key).map_err(Into::into)
                })
            })?
            .await?
    }

    /// Fetch a key from the nonconsensus column family.
    async fn get_nonconsensus(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        let span = Span::current();
        let inner = self.0.clone();
        let key: Vec<u8> = key.to_vec();
        tokio::task::Builder::new()
            .name("Snapshot::get_nonconsensus")
            .spawn_blocking(move || {
                span.in_scope(|| {
                    let nonconsensus_cf = inner
                        .db
                        .cf_handle("nonconsensus")
                        .expect("nonconsensus column family not found");
                    inner
                        .snapshot
                        .get_cf(nonconsensus_cf, key)
                        .map_err(Into::into)
                })
            })?
            .await?
    }

    async fn prefix_raw<'a>(
        &'a self,
        prefix: &'a str,
    ) -> Pin<Box<dyn Stream<Item = Result<(String, Box<[u8]>)>> + Sync + Send + 'a>> {
        let span = Span::current();
        let inner = self.0.clone();

        let mut options = rocksdb::ReadOptions::default();
        options.set_iterate_range(rocksdb::PrefixRange(prefix.as_bytes()));
        let mode = rocksdb::IteratorMode::Start;

        let (tx, rx) = mpsc::channel(10);

        // Since the JMT keys are hashed, we can't use a prefix iterator directly.
        // We need to first prefix range the key preimages column family, then use the hashed matches to fetch the values
        // from the JMT column family.
        tokio::task::Builder::new()
            .name("Snapshot::prefix_raw")
            .spawn_blocking(move || {
                span.in_scope(|| {
                    let jmt_cf = inner
                        .db
                        .cf_handle("jmt")
                        .expect("jmt column family not found");
                    let keys_cf = inner
                        .db
                        .cf_handle("jmt_keys")
                        .expect("jmt_keys column family not found");
                    let iter = inner.snapshot.iterator_cf_opt(keys_cf, options, mode);
                    for i in iter {
                        // For each key that matches the prefix, fetch the value from the JMT column family.
                        let (key_preimage, key_hash) = i?;

                        let j = inner
                            .snapshot
                            .get_pinned_cf(jmt_cf, key_hash)?
                            .expect("keys in jmt_keys should have a corresponding value in jmt");
                        let k = std::str::from_utf8(key_preimage.as_ref())?;
                        tx.blocking_send(Ok((k.to_string(), Box::from(j.as_ref()))))?;
                    }
                    Ok::<(), anyhow::Error>(())
                })
            })
            .expect("should be able to spawn_blocking");

        Box::pin(tokio_stream::wrappers::ReceiverStream::new(rx))
    }
}

/// A reader interface for rocksdb. NOTE: it is up to the caller to ensure consistency between the
/// rocksdb::DB handle and any write batches that may be applied through the writer interface.
impl TreeReader for Snapshot {
    /// Gets node given a node key. Returns `None` if the node does not exist.
    fn get_node_option(&self, node_key: &NodeKey) -> Result<Option<Node>> {
        let node_key = node_key;
        tracing::trace!(?node_key);

        let jmt_cf = self
            .0
            .db
            .cf_handle("jmt")
            .expect("jmt column family not found");
        let value = self
            .0
            .snapshot
            .get_cf(jmt_cf, &node_key.encode()?)?
            .map(|db_slice| Node::decode(&db_slice))
            .transpose()?;

        tracing::trace!(?node_key, ?value);
        Ok(value)
    }

    fn get_rightmost_leaf(&self) -> Result<Option<(NodeKey, LeafNode)>> {
        let jmt_cf = self
            .0
            .db
            .cf_handle("jmt")
            .expect("jmt column family not found");
        let mut iter = self.0.snapshot.raw_iterator_cf(jmt_cf);
        iter.seek_to_last();

        if iter.valid() {
            let node_key = NodeKey::decode(iter.key().unwrap())?;
            let node = Node::decode(iter.value().unwrap())?;

            if let Node::Leaf(leaf_node) = node {
                return Ok(Some((node_key, leaf_node)));
            }
        } else {
            // There are no keys in the database
        }

        Ok(None)
    }
}
