use std::{fs::OpenOptions, io::Write, process::Command};

use proxmox_sys::linux::procfs;

use crate::setup::KeyboardMapping;

pub fn has_min_requirements() -> Result<(), String> {
    let meminfo = procfs::read_meminfo().map_err(|err| err.to_string())?;

    if meminfo.memtotal / 1024 / 1024 < 1024 {
        return Err(concat!(
            "Less than 1 GiB of usable memory detected, installation will probably fail.\n\n",
            "See 'System Requirements' in the documentation."
        )
        .to_owned());
    }

    let cpuinfo = procfs::read_cpuinfo().map_err(|err| err.to_string())?;

    if !cpuinfo.hvm {
        return Err(concat!(
            "No support for hardware-accelerated KVM virtualization detected.\n\n",
            "Check BIOS settings for Intel VT / AMD-V / SVM."
        )
        .to_owned());
    }

    Ok(())
}

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
