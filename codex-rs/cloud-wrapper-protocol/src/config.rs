#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CloudRuntimeDefaults {
    pub mysql_max_connections: u32,
    pub owner_lease_ttl_ms: u64,
    pub owner_lease_heartbeat_ms: u64,
    pub turn_timeout_ms: u64,
    pub request_timeout_ms: u64,
}

impl Default for CloudRuntimeDefaults {
    fn default() -> Self {
        Self {
            mysql_max_connections: 20,
            owner_lease_ttl_ms: 180_000,
            owner_lease_heartbeat_ms: 2_000,
            turn_timeout_ms: 1_800_000,
            request_timeout_ms: 60_000,
        }
    }
}
