use proxmox_sys::linux::procfs;

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
