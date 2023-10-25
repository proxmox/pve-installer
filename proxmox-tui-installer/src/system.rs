use std::{fs::OpenOptions, io::Write, process::Command};

use proxmox_installer_common::setup::KeyboardMapping;

pub fn set_keyboard_layout(kmap: &KeyboardMapping) -> Result<(), String> {
    Command::new("setxkbmap")
        .args([&kmap.xkb_layout, &kmap.xkb_variant])
        .output()
        .map_err(|err| err.to_string())?;

    let mut f = OpenOptions::new()
        .write(true)
        .truncate(true)
        .open("/etc/default/keyboard")
        .map_err(|err| err.to_string())?;

    write!(
        f,
        r#"XKBLAYOUT="{}"
XKBVARIANT="{}"
BACKSPACE="guess"
"#,
        kmap.xkb_layout, kmap.xkb_variant
    )
    .map_err(|err| err.to_string())?;

    Command::new("setupcon")
        .output()
        .map_err(|err| err.to_string())?;

    Ok(())
}
