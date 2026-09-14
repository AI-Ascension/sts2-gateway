// SPDX-License-Identifier: MIT

use sts2_gateway::{
    InMemorySaveProfileRecordStore, LaunchProfileBindingPort, SaveProfileLedgerError,
    SaveProfileOperationRecord, SaveProfileRecordStore, UserDataPort, UserDataProvisioner,
    UserDataRecordStore,
};

/// Authoritative active-run admission for save-profile mutations.
///
/// `governed` is `None` while no authoritative active-run writer is configured. The attached
/// runtime then refuses every mutation instead of assuming that no run is in progress.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct SaveProfileActiveRun {
    pub(super) governed: Option<bool>,
}

impl SaveProfileActiveRun {
    /// No authoritative active-run source is configured.
    pub(super) const UNCONFIGURED: Self = Self { governed: None };

    /// True only when an authoritative source reports that a run is in progress.
    pub(super) const fn active(self) -> bool {
        matches!(self.governed, Some(true))
    }

    /// True only when an authoritative source is configured.
    pub(super) const fn configured(self) -> bool {
        self.governed.is_some()
    }
}

/// Injected save-profile adapters.
///
/// No accepted durable operation-intent store, isolated-allocation port, or launch-profile
/// binding port exists for this slice, so the attached runtime injects none of them. Every
/// mutation then fails closed with an explicit unavailable capability instead of silently
/// creating volatile in-memory state that a restart would lose.
#[derive(Default)]
pub(super) struct SaveProfileDependencies {
    pub(super) intents: Option<Box<dyn SaveProfileRecordStore + Send>>,
    pub(super) allocation_port: Option<Box<dyn UserDataPort + Send>>,
    pub(super) allocation_intents: Option<Box<dyn UserDataRecordStore + Send>>,
    pub(super) launch_profile: Option<Box<dyn LaunchProfileBindingPort + Send>>,
}

impl SaveProfileDependencies {
    /// No durable intent store, isolated-allocation port, or launch-profile binding port.
    pub(super) fn unavailable() -> Self {
        Self::default()
    }
}

/// Isolated allocation adapter set bound to the runtime operation capacity.
pub(super) type SaveProfileAllocation = UserDataProvisioner<
    Box<dyn UserDataPort + Send>,
    Box<dyn UserDataRecordStore + Send>,
    Box<dyn LaunchProfileBindingPort + Send>,
>;

/// Operation-intent storage backing the fixed save-profile ledger.
pub(super) enum SaveProfileIntentStore {
    /// Volatile read-path cache. Discovery reads carry no mutation authority, so losing this at
    /// restart is safe; no mutation is ever recorded here.
    Volatile(InMemorySaveProfileRecordStore),
    /// Injected durable store. Only this variant admits a mutation.
    Durable(Box<dyn SaveProfileRecordStore + Send>),
}

impl SaveProfileIntentStore {
    pub(super) fn from_injected(store: Option<Box<dyn SaveProfileRecordStore + Send>>) -> Self {
        match store {
            Some(store) => Self::Durable(store),
            None => Self::Volatile(InMemorySaveProfileRecordStore::default()),
        }
    }

    pub(super) const fn durable(&self) -> bool {
        matches!(self, Self::Durable(_))
    }
}

impl SaveProfileRecordStore for SaveProfileIntentStore {
    fn list(&mut self) -> Result<Vec<SaveProfileOperationRecord>, SaveProfileLedgerError> {
        match self {
            Self::Volatile(store) => store.list(),
            Self::Durable(store) => store.list(),
        }
    }

    fn insert(&mut self, record: SaveProfileOperationRecord) -> Result<(), SaveProfileLedgerError> {
        match self {
            Self::Volatile(store) => store.insert(record),
            Self::Durable(store) => store.insert(record),
        }
    }

    fn update(&mut self, record: SaveProfileOperationRecord) -> Result<(), SaveProfileLedgerError> {
        match self {
            Self::Volatile(store) => store.update(record),
            Self::Durable(store) => store.update(record),
        }
    }
}
