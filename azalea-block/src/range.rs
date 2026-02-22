use std::{
    collections::{HashSet, hash_set},
    ops::{Add, RangeInclusive},
    sync::{
        LazyLock,
        atomic::{AtomicU32, Ordering},
    },
};

use azalea_registry::{builtin::BlockKind, tags::RegistryTag};

use crate::{BlockState, block_state::BlockStateIntegerRepr};

/// Extended block state range for modded support.
/// Stores the maximum state ID that should be considered valid beyond vanilla.
/// Defaults to 0, meaning only vanilla MAX_STATE is used.
static MOD_MAX_STATE: AtomicU32 = AtomicU32::new(0);

/// Extend the valid block state range beyond vanilla's MAX_STATE.
/// This allows joining modded servers that send block state IDs above
/// the vanilla maximum.
///
/// # Arguments
/// * `new_max` - The new maximum state ID to accept
pub fn set_mod_max_state(new_max: u32) {
    MOD_MAX_STATE.store(new_max, Ordering::Relaxed);
}

/// Get the current mod-extended max state ID.
pub fn get_mod_max_state() -> u32 {
    MOD_MAX_STATE.load(Ordering::Relaxed)
}

#[derive(Clone, Debug)]
pub struct BlockStates {
    pub set: HashSet<BlockState>,
}

impl From<RangeInclusive<BlockStateIntegerRepr>> for BlockStates {
    fn from(range: RangeInclusive<BlockStateIntegerRepr>) -> Self {
        let mut set = HashSet::with_capacity((range.end() - range.start() + 1) as usize);
        for id in range {
            if BlockState::is_valid_state_extended(id) {
                set.insert(BlockState::from_raw_unchecked(id));
            }
        }
        Self { set }
    }
}

impl IntoIterator for BlockStates {
    type Item = BlockState;
    type IntoIter = hash_set::IntoIter<BlockState>;

    fn into_iter(self) -> Self::IntoIter {
        self.set.into_iter()
    }
}

impl BlockStates {
    pub fn contains(&self, state: &BlockState) -> bool {
        self.set.contains(state)
    }
}

impl Add for BlockStates {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self {
            set: self.set.union(&rhs.set).copied().collect(),
        }
    }
}

impl From<HashSet<BlockKind>> for BlockStates {
    fn from(set: HashSet<BlockKind>) -> Self {
        Self::from(&set)
    }
}

impl From<&HashSet<BlockKind>> for BlockStates {
    fn from(set: &HashSet<BlockKind>) -> Self {
        let mut block_states = HashSet::with_capacity(set.len());
        for &block in set {
            block_states.extend(BlockStates::from(block));
        }
        Self { set: block_states }
    }
}

impl From<&[BlockKind]> for BlockStates {
    fn from(arr: &[BlockKind]) -> Self {
        let mut block_states = HashSet::with_capacity(arr.len());
        for &block in arr {
            block_states.extend(BlockStates::from(block));
        }
        Self { set: block_states }
    }
}
impl<const N: usize> From<[BlockKind; N]> for BlockStates {
    fn from(arr: [BlockKind; N]) -> Self {
        Self::from(&arr[..])
    }
}
impl From<&RegistryTag<BlockKind>> for BlockStates {
    fn from(tag: &RegistryTag<BlockKind>) -> Self {
        Self::from(&**tag)
    }
}
// allows users to do like `BlockStates::from(&tags::blocks::LOGS)` instead of
// `BlockStates::from(&&tags::blocks::LOGS)`
impl From<&LazyLock<RegistryTag<BlockKind>>> for BlockStates {
    fn from(tag: &LazyLock<RegistryTag<BlockKind>>) -> Self {
        Self::from(&**tag)
    }
}
