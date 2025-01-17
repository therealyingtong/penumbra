use anyhow::Result;
use async_trait::async_trait;
use futures::Stream;
use std::{collections::BTreeMap, pin::Pin};

use crate::State;

use super::{read::prefix_raw_with_cache, StateRead, StateWrite};

/// Represents a transactional set of changes to a `State` fork,
/// implemented as a RYW cache over a `State`.
pub struct Transaction<'a> {
    /// Unwritten changes to the consensus-critical state (stored in the JMT).
    pub(crate) unwritten_changes: BTreeMap<String, Option<Vec<u8>>>,
    /// Unwritten changes to non-consensus-critical state (stored in the nonconsensus storage).
    pub(crate) nonconsensus_changes: BTreeMap<Vec<u8>, Option<Vec<u8>>>,
    state: &'a mut State,
    pub(crate) failed: bool,
    pub(crate) failure_reason: String,
}

impl<'a> Transaction<'a> {
    pub fn new(state: &'a mut State) -> Self {
        Self {
            state,
            unwritten_changes: BTreeMap::new(),
            nonconsensus_changes: BTreeMap::new(),
            failed: false,
            failure_reason: String::new(),
        }
    }

    pub fn fail(&mut self, reason: String) {
        self.failed = true;
        self.failure_reason = reason;
    }

    pub fn commit(self) -> Result<()> {
        if self.failed {
            return Err(anyhow::anyhow!("transaction failed").context(self.failure_reason));
        }

        // Write the unwritten consensus-critical changes to the state:
        self.state.unwritten_changes.extend(self.unwritten_changes);

        // Write the unwritten nonconsensus changes to the state:
        self.state
            .nonconsensus_changes
            .extend(self.nonconsensus_changes);

        Ok(())
    }
}

impl<'a> StateWrite for Transaction<'a> {
    fn put_raw(&mut self, key: String, value: jmt::OwnedValue) {
        self.unwritten_changes.insert(key, Some(value));
    }

    fn delete(&mut self, key: String) {
        self.unwritten_changes.insert(key, None);
    }

    fn delete_nonconsensus(&mut self, key: Vec<u8>) {
        self.nonconsensus_changes.insert(key, None);
    }

    fn put_nonconsensus(&mut self, key: Vec<u8>, value: Vec<u8>) {
        self.nonconsensus_changes.insert(key, Some(value));
    }
}

#[async_trait]
impl<'tx> StateRead for Transaction<'tx> {
    async fn get_raw(&self, key: &str) -> Result<Option<Vec<u8>>> {
        // If the key is available in the unwritten_changes cache, return it.
        if let Some(v) = self.unwritten_changes.get(key) {
            return Ok(v.clone());
        }

        // Otherwise, if the key is available in the state, return it.
        self.state.get_raw(key).await
    }

    async fn get_nonconsensus(&self, key: &[u8]) -> Result<Option<Vec<u8>>> {
        // If the key is available in the nonconsensus cache, return it.
        if let Some(v) = self.nonconsensus_changes.get(key) {
            return Ok(v.clone());
        }

        // Otherwise, if the key is available in the state, return it.
        self.state.get_nonconsensus(key).await
    }

    async fn prefix_raw<'a>(
        &'a self,
        prefix: &'a str,
    ) -> Pin<Box<dyn Stream<Item = Result<(String, Box<[u8]>)>> + Sync + Send + 'a>> {
        prefix_raw_with_cache(self, &self.unwritten_changes, prefix).await
    }
}
