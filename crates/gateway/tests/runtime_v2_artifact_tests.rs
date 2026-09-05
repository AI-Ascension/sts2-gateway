// SPDX-License-Identifier: MIT

use sts2_gateway::{
    RuntimeV2ArtifactFile, RuntimeV2ArtifactFiles, runtime_v2_artifact_files,
    verify_runtime_v2_artifact_files,
};

#[test]
fn copied_artifact_rejects_tampered_schema_manifest_and_golden() -> Result<(), String> {
    let base = runtime_v2_artifact_files();

    let mut tampered_schema = base.source_schema.to_vec();
    tampered_schema[0] ^= 1;
    assert!(
        verify_runtime_v2_artifact_files(RuntimeV2ArtifactFiles {
            source_schema: &tampered_schema,
            ..base
        })
        .is_err()
    );

    let mut tampered_manifest = base.manifest.to_vec();
    tampered_manifest.push(b'\n');
    assert!(
        verify_runtime_v2_artifact_files(RuntimeV2ArtifactFiles {
            manifest: &tampered_manifest,
            ..base
        })
        .is_err()
    );

    let mut tampered_golden = base.goldens[0].bytes.to_vec();
    tampered_golden.push(b'\n');
    let mut goldens = base.goldens.to_vec();
    goldens[0] = RuntimeV2ArtifactFile {
        path: goldens[0].path,
        bytes: &tampered_golden,
    };
    assert!(
        verify_runtime_v2_artifact_files(RuntimeV2ArtifactFiles {
            goldens: &goldens,
            ..base
        })
        .is_err()
    );
    Ok(())
}
