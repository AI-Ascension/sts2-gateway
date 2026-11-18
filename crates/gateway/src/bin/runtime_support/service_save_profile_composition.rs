// SPDX-License-Identifier: MIT

use std::path::Path;

use sts2_gateway::{
    FilesystemUserDataPort, InMemorySaveProfileRecordStore, LaunchProfileBindingPort,
    SaveProfileLedgerError, SaveProfileOperationRecord, SaveProfileRecordStore,
    SqliteSaveProfileOperationStore, SqliteUserDataRecordStore, UserDataPort, UserDataProvisioner,
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
/// Filesystem and durable stores exist, but the actual launch binding and authoritative producer
/// admission are not composed, so the attached runtime injects none of them. Every
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

    /// Assemble durable allocation and operation ports only with an explicitly owned
    /// deployment root and an independently accepted launch-profile owner. This is a
    /// preparation seam, not native admission: no runtime path calls it by default.
    /// Active-run authority must be composed separately before any mutation.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn from_owned_storage(
        allocation_root: &Path,
        allocation_journal: &Path,
        operation_journal: &Path,
        capacity: usize,
        launch_profile: Box<dyn LaunchProfileBindingPort + Send>,
    ) -> Result<Self, String> {
        // The two durable stores have distinct schemas and locks. Refuse shared
        // path identity before creating or opening either database.
        if allocation_journal == operation_journal
            || (allocation_journal.exists()
                && operation_journal.exists()
                && allocation_journal.canonicalize().ok() == operation_journal.canonicalize().ok())
        {
            return Err(String::from("save-profile journals must be distinct"));
        }
        // Validate the owned root before any journal can be created.
        let allocation_port = FilesystemUserDataPort::try_new(allocation_root, capacity)
            .map_err(|_| String::from("save-profile owned allocation root unavailable"))?;
        let allocation_intents = SqliteUserDataRecordStore::open(allocation_journal)
            .map_err(|_| String::from("save-profile allocation journal unavailable"))?;
        let intents = SqliteSaveProfileOperationStore::open(operation_journal)
            .map_err(|_| String::from("save-profile operation journal unavailable"))?;
        Ok(Self {
            intents: Some(Box::new(intents)),
            allocation_port: Some(Box::new(allocation_port)),
            allocation_intents: Some(Box::new(allocation_intents)),
            launch_profile: Some(launch_profile),
        })
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
