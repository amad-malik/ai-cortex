// A piece never switches the role: SET ROLE cortex_install_admin is the host path's, in cortex-db.
/* block: SET LOCAL ROLE cortex_install_admin; set_config('role', 'x', true) */
pub fn words() -> &'static str { "asset role, preset role and the reset_role_count" }
pub fn reset_role_count() -> u32 { 0 }
pub fn presetrole() -> u32 { 1 }
