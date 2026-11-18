pub const RAW: &str = r#"SET ROLE 'x'"#;
pub const ESC_QUOTE: &str = "SELECT set_config(\'role\', \'x\', true)";
pub const HEX: &str = "SELECT set_config(\x27role\x27, 1, true)";
pub const UNI: &str = "SELECT set_config(\u{27}role\u{27}, 1, true)";
pub const SPACED: &str = "SELECT  set_config ( 'role' , 'x', true)";
pub const NEWLINE_ESC: &str = "set\nrole x";
pub const CONTINUED: &str = "RESET \
        ROLE";
pub const SPLIT_WORD: &str = "SET RO\
        LE x";
pub const JOINED: &str = concat!("SET ", "LOCAL ROLE x");
pub const SESSION: &str = "set session role x; reset session authorization";
pub const NAME_BYTES: &[u8] = b"grant cortex_install_admin to y";
pub const ALIKE_1: &str = "reset_role_count and preset role and asset role";
pub const ALIKE_2: &str = "cortex_install_admin2 and xcortex_install_admin and cortex_install_administrator";
pub const ALIKE_3: &str = "set rolex and set roles and settings_role";
pub const ALIKE_4: &str = "SET ROL";
pub const ALIKE_5: &str = "set_config('rolex', 'x', true) and set_config('search_path'";
pub const LOCAL_SESSION: &str = "SET LOCAL SESSION AUTHORIZATION x";
pub const E_QUOTED: &str = "SELECT set_config(E'role', 'x', true)";
pub const DOLLAR: &str = "SELECT set_config($$role$$, 'x', true)";
pub const TAGGED: &str = "SELECT set_config($q$role$q$, 'x', true)";
pub const HEX_NEWLINE: &str = "a\x0aSET ROLE x";
pub const AFTER: &str = "SET ROLE y";
pub const PLACEHOLDER: &str = "SELECT set_config($1, $2, true) -- a bound key is not seen";
