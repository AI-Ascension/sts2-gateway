// SPDX-License-Identifier: MIT

use super::*;

pub(super) fn map_host_lease_transport_error(
    error: RecoveryControlTransportFault,
) -> HostLeaseFailure {
    match error {
        RecoveryControlTransportFault::InvalidConfiguration
        | RecoveryControlTransportFault::InvalidFrame
        | RecoveryControlTransportFault::RequestOversized => HostLeaseFailure::configuration(),
        RecoveryControlTransportFault::UnavailableBeforeWrite => HostLeaseFailure {
            status: 503,
            code: "recovery_host_lease_unavailable",
        },
        RecoveryControlTransportFault::DisconnectedAfterWrite
        | RecoveryControlTransportFault::TimeoutAfterWrite
        | RecoveryControlTransportFault::MalformedResponse => HostLeaseFailure::unknown(),
    }
}

pub(super) fn map_host_lease_response_error(error: HostLeaseFrameError) -> HostLeaseFailure {
    match error {
        // A malformed or unauthenticated response was observed only after the
        // mutation-bearing request was written, so its host-side effect is
        // unknowable and must remain pending for the idempotent retry path.
        HostLeaseFrameError::Invalid
        | HostLeaseFrameError::Oversized
        | HostLeaseFrameError::Authentication => HostLeaseFailure::unknown(),
        HostLeaseFrameError::Configuration => HostLeaseFailure::configuration(),
    }
}
