pub mod disk_checks;
pub mod options;
pub mod setup;
pub mod sysinfo;
pub mod utils;

#[cfg(feature = "http")]
pub mod http;

#[cfg(feature = "cli")]
pub mod cli;

pub const RUNTIME_DIR: &str = "/run/proxmox-installer";

/// Default placeholder value for the administrator email address.
pub const EMAIL_DEFAULT_PLACEHOLDER: &str = "mail@example.invalid";

/// Name of the executable for the first-boot hook.
pub const FIRST_BOOT_EXEC_NAME: &str = "proxmox-first-boot";

/// Maximum file size for the first-boot hook executable.
pub const FIRST_BOOT_EXEC_MAX_SIZE: usize = 1024 * 1024; // 1 MiB

/// Minimum length for the root password
pub const ROOT_PASSWORD_MIN_LENGTH: usize = 8;
