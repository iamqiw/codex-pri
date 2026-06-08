use pretty_assertions::assert_eq;

use crate::DisabledCapability;
use crate::RuntimeCapabilityProfile;
use crate::RuntimeMode;

#[test]
fn default_runtime_capability_profile_is_strictly_read_only() {
    let profile = RuntimeCapabilityProfile::strict_read_only();

    assert_eq!(profile.mode, RuntimeMode::ReadOnly);
    assert!(profile.read_only_runtime_required);
    assert_eq!(
        profile.disabled_capabilities,
        vec![
            DisabledCapability::FileWrite,
            DisabledCapability::ApplyPatch,
            DisabledCapability::LocalCommandExec,
            DisabledCapability::ProcessSpawn,
            DisabledCapability::SubAgent,
        ]
    );
}
