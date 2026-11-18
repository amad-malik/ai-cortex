pub const HOST_ROLE: &str = "cortex_install_admin";
pub fn back(role: &str) -> String { format!("SET LOCAL ROLE {role}; SELECT set_config('role', $1, true); RESET ROLE") }
