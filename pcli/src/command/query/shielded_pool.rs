use anyhow::Result;
use colored_json::prelude::*;
use jmt::KeyHash;
use penumbra_chain::{quarantined::Scheduled, CompactBlock, NoteSource};
use penumbra_component::shielded_pool::Delible;
use penumbra_crypto::Nullifier;
use penumbra_proto::Protobuf;
use penumbra_tct::Commitment;

#[derive(Debug, clap::Subcommand)]
pub enum ShieldedPool {
    /// Queries the note commitment tree anchor for a given height.
    Anchor {
        /// The height to query.
        height: u64,
    },
    /// Queries the note commitment tree's block anchor for a given height.
    BlockAnchor {
        /// The height to query.
        height: u64,
    },
    /// Queries the note commitment tree's epoch anchor for a given epoch index.
    EpochAnchor {
        /// The epoch to query.
        epoch: u64,
    },
    /// Queries the scheduled notes and nullifiers to unquarantine at a given epoch.
    Scheduled {
        /// The epoch to query.
        epoch: u64,
    },
    /// Queries the source of a given commitment.
    Commitment {
        /// The commitment to query.
        #[clap(parse(try_from_str = Commitment::parse_hex))]
        commitment: Commitment,
    },
    /// Queries the note source of a given nullifier.
    Nullifier {
        /// The nullifier to query.
        #[clap(parse(try_from_str = Nullifier::parse_hex))]
        nullifier: Nullifier,
    },
    /// Queries the note source of a given quarantined nullifier.
    QuarantinedNullifier {
        /// The nullifier to query.
        #[clap(parse(try_from_str = Nullifier::parse_hex))]
        nullifier: Nullifier,
    },
    /// Queries the compact block at a given height.
    CompactBlock { height: u64 },
}

impl ShieldedPool {
    pub fn key_hash(&self) -> KeyHash {
        use penumbra_component::shielded_pool::state_key;
        match self {
            ShieldedPool::Anchor { height } => state_key::anchor_by_height(*height),
            ShieldedPool::BlockAnchor { height } => state_key::block_anchor_by_height(*height),
            ShieldedPool::EpochAnchor { epoch } => state_key::epoch_anchor_by_index(*epoch),
            ShieldedPool::CompactBlock { height } => state_key::compact_block(*height),
            ShieldedPool::Scheduled { epoch } => state_key::scheduled_to_apply(*epoch),
            ShieldedPool::Commitment { commitment } => state_key::note_source(*commitment),
            ShieldedPool::Nullifier { nullifier } => state_key::spent_nullifier_lookup(*nullifier),
            ShieldedPool::QuarantinedNullifier { nullifier } => {
                state_key::quarantined_spent_nullifier_lookup(*nullifier)
            }
        }
    }

    pub fn display_value(&self, bytes: &[u8]) -> Result<()> {
        let json = match self {
            ShieldedPool::Anchor { .. } => {
                let anchor = penumbra_tct::Root::decode(bytes)?;
                serde_json::to_string_pretty(&anchor)?
            }
            ShieldedPool::BlockAnchor { .. } => {
                let anchor = penumbra_tct::builder::block::Root::decode(bytes)?;
                serde_json::to_string_pretty(&anchor)?
            }
            ShieldedPool::EpochAnchor { .. } => {
                let anchor = penumbra_tct::builder::epoch::Root::decode(bytes)?;
                serde_json::to_string_pretty(&anchor)?
            }
            ShieldedPool::CompactBlock { .. } => {
                let compact_block = CompactBlock::decode(bytes)?;
                serde_json::to_string_pretty(&compact_block)?
            }
            ShieldedPool::Scheduled { .. } => {
                let notes = Scheduled::decode(bytes)?;
                serde_json::to_string_pretty(&notes.scheduled)?
            }
            ShieldedPool::Commitment { .. } => {
                let note_source = Delible::<NoteSource>::decode(bytes)?;
                serde_json::to_string_pretty(&note_source)?
            }
            ShieldedPool::Nullifier { .. } => {
                let note_source = NoteSource::decode(bytes)?;
                serde_json::to_string_pretty(&note_source)?
            }
            ShieldedPool::QuarantinedNullifier { .. } => {
                let note_source = Delible::<NoteSource>::decode(bytes)?;
                serde_json::to_string_pretty(&note_source)?
            }
        };
        println!("{}", json.to_colored_json_auto()?);
        Ok(())
    }
}
