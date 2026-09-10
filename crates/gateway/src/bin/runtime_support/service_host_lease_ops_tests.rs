// SPDX-License-Identifier: MIT

use super::*;

#[test]
fn malformed_post_write_response_is_an_unknown_outcome() {
    assert_eq!(
        map_host_lease_transport_error(RecoveryControlTransportFault::MalformedResponse),
        HostLeaseFailure::unknown()
    );
    assert_eq!(
        map_host_lease_response_error(HostLeaseFrameError::Invalid),
        HostLeaseFailure::unknown()
    );
}
