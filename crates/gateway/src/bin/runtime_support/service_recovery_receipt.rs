// SPDX-License-Identifier: MIT

//! Host receipts are consumed by `service_recovery_dispatch` only after crossing the recovery
//! control boundary. This module remains as a compatibility path for the existing module layout;
//! it intentionally contains no local receipt or witness constructor.
