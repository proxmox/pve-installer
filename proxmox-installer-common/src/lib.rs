pub mod disk_checks;
pub mod options;
pub mod setup;
pub mod sysinfo;
pub mod utils;

#[cfg(feature = "http")]
pub mod http;

pub const RUNTIME_DIR: &str = "/run/proxmox-installer";

/// Default placeholder value for the administrator email address.
pub const EMAIL_DEFAULT_PLACEHOLDER: &str = "mail@example.invalid";
