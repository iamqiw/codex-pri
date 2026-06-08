use serde::Deserialize;
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeMode {
    ReadOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisabledCapability {
    FileWrite,
    ApplyPatch,
    LocalCommandExec,
    ProcessSpawn,
    SubAgent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeCapabilityProfile {
    pub mode: RuntimeMode,
    pub read_only_runtime_required: bool,
    pub disabled_capabilities: Vec<DisabledCapability>,
}

impl RuntimeCapabilityProfile {
    pub fn strict_read_only() -> Self {
        Self {
            mode: RuntimeMode::ReadOnly,
            read_only_runtime_required: true,
            disabled_capabilities: vec![
                DisabledCapability::FileWrite,
                DisabledCapability::ApplyPatch,
                DisabledCapability::LocalCommandExec,
                DisabledCapability::ProcessSpawn,
                DisabledCapability::SubAgent,
            ],
        }
    }
}
