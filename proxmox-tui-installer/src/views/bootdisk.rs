use std::{cell::RefCell, collections::HashSet, marker::PhantomData, rc::Rc};

use cursive::{
    view::{Nameable, Resizable, ViewWrapper},
    views::{
        Button, Dialog, DummyView, LinearLayout, NamedView, PaddedView, Panel, ScrollView,
        SelectView, TextView,
    },
    Cursive, View,
};

use super::{DiskSizeEditView, FormView, IntegerEditView};
use crate::{
    options::{
        AdvancedBootdiskOptions, BootdiskOptions, BtrfsBootdiskOptions, BtrfsRaidLevel, Disk,
        FsType, LvmBootdiskOptions, ZfsBootdiskOptions, ZfsRaidLevel, FS_TYPES,
        ZFS_CHECKSUM_OPTIONS, ZFS_COMPRESS_OPTIONS,
    },
    setup::{BootType, ProductConfig},
};
use crate::{setup::ProxmoxProduct, InstallerState};

pub struct BootdiskOptionsView {
    view: LinearLayout,
    advanced_options: Rc<RefCell<BootdiskOptions>>,
    boot_type: BootType,
}

impl BootdiskOptionsView {
    pub fn new(siv: &mut Cursive, disks: &[Disk], options: &BootdiskOptions) -> Self {
        let bootdisk_form = FormView::new()
            .child(
                "Target harddisk",
                SelectView::new()
                    .popup()
                    .with_all(disks.iter().map(|d| (d.to_string(), d.clone()))),
            )
            .with_name("bootdisk-options-target-disk");

        let product_conf = siv
            .user_data::<InstallerState>()
            .map(|state| state.setup_info.config.clone())
            .unwrap(); // Safety: InstallerState must always be set

        let advanced_options = Rc::new(RefCell::new(options.clone()));

        let advanced_button = LinearLayout::horizontal()
            .child(DummyView.full_width())
            .child(Button::new("Advanced options", {
                let disks = disks.to_owned();
                let options = advanced_options.clone();
                move |siv| {
                    siv.add_layer(advanced_options_view(
                        &disks,
                        options.clone(),
                        product_conf.clone(),
                    ));
                }
            }));

        let view = LinearLayout::vertical()
            .child(bootdisk_form)
            .child(DummyView)
            .child(advanced_button);

        let boot_type = siv
            .user_data::<InstallerState>()
            .map(|state| state.runtime_info.boot_type)
            .unwrap_or(BootType::Bios);

        Self {
            view,
            advanced_options,
            boot_type,
        }
    }

    pub fn get_values(&mut self) -> Result<BootdiskOptions, String> {
        let mut options = (*self.advanced_options).clone().into_inner();

        if [FsType::Ext4, FsType::Xfs].contains(&options.fstype) {
            let disk = self
                .view
                .get_child_mut(0)
                .and_then(|v| v.downcast_mut::<NamedView<FormView>>())
                .map(NamedView::<FormView>::get_mut)
                .and_then(|v| v.get_value::<SelectView<Disk>, _>(0))
                .ok_or("failed to retrieve bootdisk")?;

            options.disks = vec![disk];
        }

        check_disks_4kn_legacy_boot(self.boot_type, &options.disks)?;
        Ok(options)
    }
}

impl ViewWrapper for BootdiskOptionsView {
    cursive::wrap_impl!(self.view: LinearLayout);
}

struct AdvancedBootdiskOptionsView {
    view: LinearLayout,
}

impl AdvancedBootdiskOptionsView {
    fn new(disks: &[Disk], options: &BootdiskOptions, product_conf: ProductConfig) -> Self {
        let filter_btrfs =
            |fstype: &&FsType| -> bool { product_conf.enable_btrfs || !fstype.is_btrfs() };

        let fstype_select = SelectView::new()
            .popup()
            .with_all(
                FS_TYPES
                    .iter()
                    .filter(filter_btrfs)
                    .map(|t| (t.to_string(), *t)),
            )
            .selected(
                FS_TYPES
                    .iter()
                    .filter(filter_btrfs)
                    .position(|t| *t == options.fstype)
                    .unwrap_or_default(),
            )
            .on_submit({
                let disks = disks.to_owned();
                move |siv, fstype| Self::fstype_on_submit(siv, &disks, fstype)
            });

        let mut view = LinearLayout::vertical()
            .child(DummyView.full_width())
            .child(FormView::new().child("Filesystem", fstype_select))
            .child(DummyView.full_width());

        match &options.advanced {
            AdvancedBootdiskOptions::Lvm(lvm) => view.add_child(LvmBootdiskOptionsView::new(
                lvm,
                product_conf.product == ProxmoxProduct::PVE,
            )),
            AdvancedBootdiskOptions::Zfs(zfs) => {
                view.add_child(ZfsBootdiskOptionsView::new(disks, zfs))
            }
            AdvancedBootdiskOptions::Btrfs(btrfs) => {
                view.add_child(BtrfsBootdiskOptionsView::new(disks, btrfs))
            }
        };

        Self { view }
    }

    fn fstype_on_submit(siv: &mut Cursive, disks: &[Disk], fstype: &FsType) {
        let is_pve = siv
            .user_data::<InstallerState>()
            .map(|state| state.setup_info.config.product == ProxmoxProduct::PVE)
            .unwrap_or_default();

        siv.call_on_name("advanced-bootdisk-options-dialog", |view: &mut Dialog| {
            if let Some(AdvancedBootdiskOptionsView { view }) =
                view.get_content_mut().downcast_mut()
            {
                view.remove_child(3);
                match fstype {
                    FsType::Ext4 | FsType::Xfs => view.add_child(LvmBootdiskOptionsView::new(
                        &LvmBootdiskOptions::defaults_from(&disks[0]),
                        is_pve,
                    )),
                    FsType::Zfs(_) => view.add_child(ZfsBootdiskOptionsView::new(
                        disks,
                        &ZfsBootdiskOptions::defaults_from(disks),
                    )),
                    FsType::Btrfs(_) => view.add_child(BtrfsBootdiskOptionsView::new(
                        disks,
                        &BtrfsBootdiskOptions::defaults_from(disks),
                    )),
                }
            }
        });

        siv.call_on_name(
            "bootdisk-options-target-disk",
            |view: &mut FormView| match fstype {
                FsType::Ext4 | FsType::Xfs => {
                    view.replace_child(
                        0,
                        SelectView::new()
                            .popup()
                            .with_all(disks.iter().map(|d| (d.to_string(), d.clone()))),
                    );
                }
                other => view.replace_child(0, TextView::new(other.to_string())),
            },
        );
    }

    fn get_values(&mut self) -> Result<BootdiskOptions, String> {
        let fstype = self
            .view
            .get_child(1)
            .and_then(|v| v.downcast_ref::<FormView>())
            .and_then(|v| v.get_value::<SelectView<FsType>, _>(0))
            .ok_or("Failed to retrieve filesystem type".to_owned())?;

        let advanced = self
            .view
            .get_child_mut(3)
            .ok_or("Failed to retrieve advanced bootdisk options view".to_owned())?;

        if let Some(view) = advanced.downcast_mut::<LvmBootdiskOptionsView>() {
            let advanced = view
                .get_values()
                .map(AdvancedBootdiskOptions::Lvm)
                .ok_or("Failed to retrieve advanced bootdisk options")?;

            Ok(BootdiskOptions {
                disks: vec![],
                fstype,
                advanced,
            })
        } else if let Some(view) = advanced.downcast_mut::<ZfsBootdiskOptionsView>() {
            let (disks, advanced) = view
                .get_values()
                .ok_or("Failed to retrieve advanced bootdisk options")?;

            if let FsType::Zfs(level) = fstype {
                check_zfs_raid_config(level, &disks).map_err(|err| format!("{fstype}: {err}"))?;
            }

            Ok(BootdiskOptions {
                disks,
                fstype,
                advanced: AdvancedBootdiskOptions::Zfs(advanced),
            })
        } else if let Some(view) = advanced.downcast_mut::<BtrfsBootdiskOptionsView>() {
            let (disks, advanced) = view
                .get_values()
                .ok_or("Failed to retrieve advanced bootdisk options")?;

            if let FsType::Btrfs(level) = fstype {
                check_btrfs_raid_config(level, &disks).map_err(|err| format!("{fstype}: {err}"))?;
            }

            Ok(BootdiskOptions {
                disks,
                fstype,
                advanced: AdvancedBootdiskOptions::Btrfs(advanced),
            })
        } else {
            Err("Invalid bootdisk view state".to_owned())
        }
    }
}

impl ViewWrapper for AdvancedBootdiskOptionsView {
    cursive::wrap_impl!(self.view: LinearLayout);
}

struct LvmBootdiskOptionsView {
    view: FormView,
    has_extra_fields: bool,
}

impl LvmBootdiskOptionsView {
    fn new(options: &LvmBootdiskOptions, show_extra_fields: bool) -> Self {
        // TODO: Set maximum accordingly to disk size
        let view = FormView::new()
            .child(
                "Total size",
                DiskSizeEditView::new()
                    .content(options.total_size)
                    .max_value(options.total_size),
            )
            .child(
                "Swap size",
                DiskSizeEditView::new_emptyable().content_maybe(options.swap_size),
            )
            .child_conditional(
                show_extra_fields,
                "Maximum root volume size",
                DiskSizeEditView::new_emptyable().content_maybe(options.max_root_size),
            )
            .child_conditional(
                show_extra_fields,
                "Maximum data volume size",
                DiskSizeEditView::new_emptyable().content_maybe(options.max_data_size),
            )
            .child(
                "Minimum free LVM space",
                DiskSizeEditView::new_emptyable().content_maybe(options.min_lvm_free),
            );

        Self {
            view,
            has_extra_fields: show_extra_fields,
        }
    }

    fn get_values(&mut self) -> Option<LvmBootdiskOptions> {
        let min_lvm_free_id = if self.has_extra_fields { 4 } else { 2 };

        let max_root_size = self
            .has_extra_fields
            .then(|| self.view.get_value::<DiskSizeEditView, _>(2))
            .flatten();
        let max_data_size = self
            .has_extra_fields
            .then(|| self.view.get_value::<DiskSizeEditView, _>(3))
            .flatten();

        Some(LvmBootdiskOptions {
            total_size: self.view.get_value::<DiskSizeEditView, _>(0)?,
            swap_size: self.view.get_value::<DiskSizeEditView, _>(1),
            max_root_size,
            max_data_size,
            min_lvm_free: self.view.get_value::<DiskSizeEditView, _>(min_lvm_free_id),
        })
    }
}

impl ViewWrapper for LvmBootdiskOptionsView {
    cursive::wrap_impl!(self.view: FormView);
}

struct MultiDiskOptionsView<T> {
    view: LinearLayout,
    phantom: PhantomData<T>,
}

impl<T: View> MultiDiskOptionsView<T> {
    fn new(avail_disks: &[Disk], selected_disks: &[usize], options_view: T) -> Self {
        let mut selectable_disks = avail_disks
            .iter()
            .map(|d| (d.to_string(), Some(d.clone())))
            .collect::<Vec<(String, Option<Disk>)>>();

        selectable_disks.push(("-- do not use --".to_owned(), None));

        let mut disk_form = FormView::new();
        for (i, _) in avail_disks.iter().enumerate() {
            disk_form.add_child(
                &format!("Harddisk {i}"),
                SelectView::new()
                    .popup()
                    .with_all(selectable_disks.clone())
                    .selected(selected_disks[i]),
            );
        }

        let mut disk_select_view = LinearLayout::vertical()
            .child(TextView::new("Disk setup").center())
            .child(DummyView)
            .child(ScrollView::new(disk_form.with_name("multidisk-disk-form")));

        if avail_disks.len() > 3 {
            let do_not_use_index = selectable_disks.len() - 1;
            let deselect_all_button = Button::new("Deselect all", move |siv| {
                siv.call_on_name("multidisk-disk-form", |view: &mut FormView| {
                    view.call_on_childs(&|v: &mut SelectView<Option<Disk>>| {
                        // As there is no .on_select() callback defined on the
                        // SelectView's, the returned callback here can be safely
                        // ignored.
                        v.set_selection(do_not_use_index);
                    });
                });
            });

            disk_select_view.add_child(PaddedView::lrtb(
                0,
                0,
                1,
                0,
                LinearLayout::horizontal()
                    .child(DummyView.full_width())
                    .child(deselect_all_button),
            ));
        }

        let options_view = LinearLayout::vertical()
            .child(TextView::new("Advanced options").center())
            .child(DummyView)
            .child(options_view);

        let view = LinearLayout::horizontal()
            .child(disk_select_view)
            .child(DummyView.fixed_width(3))
            .child(options_view);

        Self {
            view: LinearLayout::vertical().child(view),
            phantom: PhantomData,
        }
    }

    fn top_panel(mut self, view: impl View) -> Self {
        if self.has_top_panel() {
            self.view.remove_child(0);
        }

        self.view.insert_child(0, Panel::new(view));
        self
    }

    ///
    /// This function returns a tuple of vectors. The first vector contains the currently selected
    /// disks in order of their selection slot. Empty slots are filtered out. The second vector
    /// contains indices of each slot's selection, which enables us to restore the selection even
    /// for empty slots.
    ///
    fn get_disks_and_selection(&mut self) -> Option<(Vec<Disk>, Vec<usize>)> {
        let mut disks = vec![];
        let view_top_index = usize::from(self.has_top_panel());

        let disk_form = self
            .view
            .get_child_mut(view_top_index)?
            .downcast_mut::<LinearLayout>()?
            .get_child_mut(0)?
            .downcast_mut::<LinearLayout>()?
            .get_child_mut(2)?
            .downcast_mut::<ScrollView<NamedView<FormView>>>()?
            .get_inner_mut()
            .get_mut();

        let mut selected_disks = Vec::new();

        for i in 0..disk_form.len() {
            let disk = disk_form.get_value::<SelectView<Option<Disk>>, _>(i)?;

            // `None` means no disk was selected for this slot
            if let Some(disk) = disk {
                disks.push(disk);
            }

            selected_disks.push(
                disk_form
                    .get_child::<SelectView<Option<Disk>>>(i)?
                    .selected_id()?,
            );
        }

        Some((disks, selected_disks))
    }

    fn inner_mut(&mut self) -> Option<&mut T> {
        let view_top_index = usize::from(self.has_top_panel());

        self.view
            .get_child_mut(view_top_index)?
            .downcast_mut::<LinearLayout>()?
            .get_child_mut(2)?
            .downcast_mut::<LinearLayout>()?
            .get_child_mut(2)?
            .downcast_mut::<T>()
    }

    fn has_top_panel(&self) -> bool {
        // The root view should only ever have one or two children
        assert!([1, 2].contains(&self.view.len()));

        self.view.len() == 2
    }
}

impl<T: 'static> ViewWrapper for MultiDiskOptionsView<T> {
    cursive::wrap_impl!(self.view: LinearLayout);
}

struct BtrfsBootdiskOptionsView {
    view: MultiDiskOptionsView<FormView>,
}

impl BtrfsBootdiskOptionsView {
    fn new(disks: &[Disk], options: &BtrfsBootdiskOptions) -> Self {
        let view = MultiDiskOptionsView::new(
            disks,
            &options.selected_disks,
            FormView::new().child("hdsize", DiskSizeEditView::new().content(options.disk_size)),
        )
        .top_panel(TextView::new("Btrfs integration is a technology preview!").center());

        Self { view }
    }

    fn get_values(&mut self) -> Option<(Vec<Disk>, BtrfsBootdiskOptions)> {
        let (disks, selected_disks) = self.view.get_disks_and_selection()?;
        let disk_size = self.view.inner_mut()?.get_value::<DiskSizeEditView, _>(0)?;

        Some((
            disks,
            BtrfsBootdiskOptions {
                disk_size,
                selected_disks,
            },
        ))
    }
}

impl ViewWrapper for BtrfsBootdiskOptionsView {
    cursive::wrap_impl!(self.view: MultiDiskOptionsView<FormView>);
}

struct ZfsBootdiskOptionsView {
    view: MultiDiskOptionsView<FormView>,
}

impl ZfsBootdiskOptionsView {
    // TODO: Re-apply previous disk selection from `options` correctly
    fn new(disks: &[Disk], options: &ZfsBootdiskOptions) -> Self {
        let inner = FormView::new()
            .child("ashift", IntegerEditView::new().content(options.ashift))
            .child(
                "compress",
                SelectView::new()
                    .popup()
                    .with_all(ZFS_COMPRESS_OPTIONS.iter().map(|o| (o.to_string(), *o)))
                    .selected(
                        ZFS_COMPRESS_OPTIONS
                            .iter()
                            .position(|o| *o == options.compress)
                            .unwrap_or_default(),
                    ),
            )
            .child(
                "checksum",
                SelectView::new()
                    .popup()
                    .with_all(ZFS_CHECKSUM_OPTIONS.iter().map(|o| (o.to_string(), *o)))
                    .selected(
                        ZFS_CHECKSUM_OPTIONS
                            .iter()
                            .position(|o| *o == options.checksum)
                            .unwrap_or_default(),
                    ),
            )
            .child("copies", IntegerEditView::new().content(options.copies))
            .child("hdsize", DiskSizeEditView::new().content(options.disk_size));

        let view = MultiDiskOptionsView::new(disks, &options.selected_disks, inner)
            .top_panel(TextView::new(
                "ZFS is not compatible with hardware RAID controllers, for details see the documentation."
            ).center());

        Self { view }
    }

    fn get_values(&mut self) -> Option<(Vec<Disk>, ZfsBootdiskOptions)> {
        let (disks, selected_disks) = self.view.get_disks_and_selection()?;
        let view = self.view.inner_mut()?;

        let ashift = view.get_value::<IntegerEditView, _>(0)?;
        let compress = view.get_value::<SelectView<_>, _>(1)?;
        let checksum = view.get_value::<SelectView<_>, _>(2)?;
        let copies = view.get_value::<IntegerEditView, _>(3)?;
        let disk_size = view.get_value::<DiskSizeEditView, _>(4)?;

        Some((
            disks,
            ZfsBootdiskOptions {
                ashift,
                compress,
                checksum,
                copies,
                disk_size,
                selected_disks,
            },
        ))
    }
}

impl ViewWrapper for ZfsBootdiskOptionsView {
    cursive::wrap_impl!(self.view: MultiDiskOptionsView<FormView>);
}

fn advanced_options_view(
    disks: &[Disk],
    options: Rc<RefCell<BootdiskOptions>>,
    product_conf: ProductConfig,
) -> impl View {
    Dialog::around(AdvancedBootdiskOptionsView::new(
        disks,
        &(*options).borrow(),
        product_conf,
    ))
    .title("Advanced bootdisk options")
    .button("Ok", {
        let options_ref = options.clone();
        move |siv| {
            let options = siv
                .call_on_name("advanced-bootdisk-options-dialog", |view: &mut Dialog| {
                    view.get_content_mut()
                        .downcast_mut::<AdvancedBootdiskOptionsView>()
                        .map(AdvancedBootdiskOptionsView::get_values)
                })
                .flatten();

            let options = match options {
                Some(Ok(options)) => options,
                Some(Err(err)) => {
                    siv.add_layer(Dialog::info(err));
                    return;
                }
                None => {
                    siv.add_layer(Dialog::info("Failed to retrieve bootdisk options view"));
                    return;
                }
            };

            if let Err(duplicate) = check_for_duplicate_disks(&options.disks) {
                siv.add_layer(Dialog::info(format!(
                    "Cannot select same disk twice: {duplicate}"
                )));
                return;
            }

            siv.pop_layer();
            *(*options_ref).borrow_mut() = options;
        }
    })
    .with_name("advanced-bootdisk-options-dialog")
    .max_size((120, 40))
}

/// Checks a list of disks for duplicate entries, using their index as key.
///
/// # Arguments
///
/// * `disks` - A list of disks to check for duplicates.
fn check_for_duplicate_disks(disks: &[Disk]) -> Result<(), &Disk> {
    let mut set = HashSet::new();

    for disk in disks {
        if !set.insert(&disk.index) {
            return Err(disk);
        }
    }

    Ok(())
}

/// Simple wrapper which returns an descriptive error if the list of disks is too short.
///
/// # Arguments
///
/// * `disks` - A list of disks to check the lenght of.
/// * `min` - Minimum number of disks
fn check_raid_min_disks(disks: &[Disk], min: usize) -> Result<(), String> {
    if disks.len() < min {
        Err(format!("Need at least {min} disks"))
    } else {
        Ok(())
    }
}

/// Checks all disks for legacy BIOS boot compatibility and reports an error as appropriate. 4Kn
/// disks are generally broken with legacy BIOS and cannot be booted from.
///
/// # Arguments
///
/// * `runinfo` - `RuntimeInfo` instance of currently running system
/// * `disks` - List of disks designated as bootdisk targets.
fn check_disks_4kn_legacy_boot(boot_type: BootType, disks: &[Disk]) -> Result<(), &str> {
    let is_blocksize_4096 = |disk: &Disk| disk.block_size.map(|s| s == 4096).unwrap_or(false);

    if boot_type == BootType::Bios && disks.iter().any(is_blocksize_4096) {
        return Err("Booting from 4Kn drive in legacy BIOS mode is not supported.");
    }

    Ok(())
}

/// Checks whether a user-supplied ZFS RAID setup is valid or not, such as disk sizes andminimum
/// number of disks.
///
/// # Arguments
///
/// * `level` - The targeted ZFS RAID level by the user.
/// * `disks` - List of disks designated as RAID targets.
fn check_zfs_raid_config(level: ZfsRaidLevel, disks: &[Disk]) -> Result<(), String> {
    // See also Proxmox/Install.pm:get_zfs_raid_setup()

    let check_mirror_size = |disk1: &Disk, disk2: &Disk| {
        if (disk1.size - disk2.size).abs() > disk1.size / 10. {
            Err(format!(
                "Mirrored disks must have same size:\n\n  * {disk1}\n  * {disk2}"
            ))
        } else {
            Ok(())
        }
    };

    match level {
        ZfsRaidLevel::Raid0 => check_raid_min_disks(disks, 1)?,
        ZfsRaidLevel::Raid1 => {
            check_raid_min_disks(disks, 2)?;
            for disk in disks {
                check_mirror_size(&disks[0], disk)?;
            }
        }
        ZfsRaidLevel::Raid10 => {
            check_raid_min_disks(disks, 4)?;
            // Pairs need to have the same size
            for i in (0..disks.len()).step_by(2) {
                check_mirror_size(&disks[i], &disks[i + 1])?;
            }
        }
        // For RAID-Z: minimum disks number is level + 2
        ZfsRaidLevel::RaidZ => {
            check_raid_min_disks(disks, 3)?;
            for disk in disks {
                check_mirror_size(&disks[0], disk)?;
            }
        }
        ZfsRaidLevel::RaidZ2 => {
            check_raid_min_disks(disks, 4)?;
            for disk in disks {
                check_mirror_size(&disks[0], disk)?;
            }
        }
        ZfsRaidLevel::RaidZ3 => {
            check_raid_min_disks(disks, 5)?;
            for disk in disks {
                check_mirror_size(&disks[0], disk)?;
            }
        }
    }

    Ok(())
}

/// Checks whether a user-supplied Btrfs RAID setup is valid or not, such as minimum
/// number of disks.
///
/// # Arguments
///
/// * `level` - The targeted Btrfs RAID level by the user.
/// * `disks` - List of disks designated as RAID targets.
fn check_btrfs_raid_config(level: BtrfsRaidLevel, disks: &[Disk]) -> Result<(), String> {
    // See also Proxmox/Install.pm:get_btrfs_raid_setup()

    match level {
        BtrfsRaidLevel::Raid0 => check_raid_min_disks(disks, 1)?,
        BtrfsRaidLevel::Raid1 => check_raid_min_disks(disks, 2)?,
        BtrfsRaidLevel::Raid10 => check_raid_min_disks(disks, 4)?,
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_disk(index: usize) -> Disk {
        Disk {
            index: index.to_string(),
            path: format!("/dev/dummy{index}"),
            model: Some("Dummy disk".to_owned()),
            size: 1024. * 1024. * 1024. * 8.,
            block_size: Some(512),
        }
    }

    fn dummy_disks(num: usize) -> Vec<Disk> {
        (0..num).map(dummy_disk).collect()
    }

    #[test]
    fn duplicate_disks() {
        assert!(check_for_duplicate_disks(&dummy_disks(2)).is_ok());
        assert_eq!(
            check_for_duplicate_disks(&[
                dummy_disk(0),
                dummy_disk(1),
                dummy_disk(2),
                dummy_disk(2),
                dummy_disk(3),
            ]),
            Err(&dummy_disk(2)),
        );
    }

    #[test]
    fn raid_min_disks() {
        let disks = dummy_disks(10);

        assert!(check_raid_min_disks(&disks[..1], 2).is_err());
        assert!(check_raid_min_disks(&disks[..1], 1).is_ok());
        assert!(check_raid_min_disks(&disks, 1).is_ok());
    }

    #[test]
    fn bios_boot_compat_4kn() {
        for i in 0..10 {
            let mut disks = dummy_disks(10);
            disks[i].block_size = Some(4096);

            // Must fail if /any/ of the disks are 4Kn
            assert!(check_disks_4kn_legacy_boot(BootType::Bios, &disks).is_err());
            // For UEFI, we allow it for every configuration
            assert!(check_disks_4kn_legacy_boot(BootType::Efi, &disks).is_ok());
        }
    }

    #[test]
    fn btrfs_raid() {
        let disks = dummy_disks(10);

        assert!(check_btrfs_raid_config(BtrfsRaidLevel::Raid0, &[]).is_err());
        assert!(check_btrfs_raid_config(BtrfsRaidLevel::Raid0, &disks[..1]).is_ok());
        assert!(check_btrfs_raid_config(BtrfsRaidLevel::Raid0, &disks).is_ok());

        assert!(check_btrfs_raid_config(BtrfsRaidLevel::Raid1, &[]).is_err());
        assert!(check_btrfs_raid_config(BtrfsRaidLevel::Raid1, &disks[..1]).is_err());
        assert!(check_btrfs_raid_config(BtrfsRaidLevel::Raid1, &disks[..2]).is_ok());
        assert!(check_btrfs_raid_config(BtrfsRaidLevel::Raid1, &disks).is_ok());

        assert!(check_btrfs_raid_config(BtrfsRaidLevel::Raid10, &[]).is_err());
        assert!(check_btrfs_raid_config(BtrfsRaidLevel::Raid10, &disks[..3]).is_err());
        assert!(check_btrfs_raid_config(BtrfsRaidLevel::Raid10, &disks[..4]).is_ok());
        assert!(check_btrfs_raid_config(BtrfsRaidLevel::Raid10, &disks).is_ok());
    }

    #[test]
    fn zfs_raid() {
        let disks = dummy_disks(10);

        assert!(check_zfs_raid_config(ZfsRaidLevel::Raid0, &[]).is_err());
        assert!(check_zfs_raid_config(ZfsRaidLevel::Raid0, &disks[..1]).is_ok());
        assert!(check_zfs_raid_config(ZfsRaidLevel::Raid0, &disks).is_ok());

        assert!(check_zfs_raid_config(ZfsRaidLevel::Raid1, &[]).is_err());
        assert!(check_zfs_raid_config(ZfsRaidLevel::Raid1, &disks[..2]).is_ok());
        assert!(check_zfs_raid_config(ZfsRaidLevel::Raid1, &disks).is_ok());

        assert!(check_zfs_raid_config(ZfsRaidLevel::Raid10, &[]).is_err());
        assert!(check_zfs_raid_config(ZfsRaidLevel::Raid10, &dummy_disks(4)).is_ok());
        assert!(check_zfs_raid_config(ZfsRaidLevel::Raid10, &disks).is_ok());

        assert!(check_zfs_raid_config(ZfsRaidLevel::RaidZ, &[]).is_err());
        assert!(check_zfs_raid_config(ZfsRaidLevel::RaidZ, &disks[..2]).is_err());
        assert!(check_zfs_raid_config(ZfsRaidLevel::RaidZ, &disks[..3]).is_ok());
        assert!(check_zfs_raid_config(ZfsRaidLevel::RaidZ, &disks).is_ok());

        assert!(check_zfs_raid_config(ZfsRaidLevel::RaidZ2, &[]).is_err());
        assert!(check_zfs_raid_config(ZfsRaidLevel::RaidZ2, &disks[..3]).is_err());
        assert!(check_zfs_raid_config(ZfsRaidLevel::RaidZ2, &disks[..4]).is_ok());
        assert!(check_zfs_raid_config(ZfsRaidLevel::RaidZ2, &disks).is_ok());

        assert!(check_zfs_raid_config(ZfsRaidLevel::RaidZ3, &[]).is_err());
        assert!(check_zfs_raid_config(ZfsRaidLevel::RaidZ3, &disks[..4]).is_err());
        assert!(check_zfs_raid_config(ZfsRaidLevel::RaidZ3, &disks[..5]).is_ok());
        assert!(check_zfs_raid_config(ZfsRaidLevel::RaidZ3, &disks).is_ok());
    }
}
