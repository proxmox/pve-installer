use std::{
    marker::PhantomData,
    sync::{Arc, Mutex},
};

use cursive::{
    Cursive, Vec2, View,
    view::{Nameable, Resizable, ViewWrapper},
    views::{
        Button, Dialog, DummyView, LinearLayout, NamedView, PaddedView, Panel, ScrollView,
        SelectView, TextView, ViewRef,
    },
};

use super::{DiskSizeEditView, FormView, IntegerEditView, TabbedView};
use crate::InstallerState;
use crate::options::FS_TYPES;

use proxmox_installer_common::{
    disk_checks::{
        check_btrfs_raid_config, check_disks_4kn_legacy_boot, check_for_duplicate_disks,
        check_zfs_raid_config,
    },
    options::{
        AdvancedBootdiskOptions, BTRFS_COMPRESS_OPTIONS, BootdiskOptions, BtrfsBootdiskOptions,
        Disk, FsType, LvmBootdiskOptions, ZFS_CHECKSUM_OPTIONS, ZFS_COMPRESS_OPTIONS,
        ZfsBootdiskOptions,
    },
    setup::{BootType, ProductConfig, ProxmoxProduct, RuntimeInfo},
};

/// OpenZFS specifies 64 MiB as the absolute minimum:
/// <https://openzfs.github.io/openzfs-docs/Performance%20and%20Tuning/Module%20Parameters.html#zfs-arc-max>
const ZFS_ARC_MIN_SIZE_MIB: usize = 64; // MiB

/// Convenience wrapper when needing to take a (interior-mutable) reference to `BootdiskOptions`.
pub type BootdiskOptionsRef = Arc<Mutex<BootdiskOptions>>;

pub struct BootdiskOptionsView {
    view: LinearLayout,
    advanced_options: BootdiskOptionsRef,
    boot_type: BootType,
}

impl BootdiskOptionsView {
    pub fn new(siv: &mut Cursive, runinfo: &RuntimeInfo, options: &BootdiskOptions) -> Self {
        let advanced_options = Arc::new(Mutex::new(options.clone()));

        let bootdisk_form = FormView::new()
            .child(
                "Target harddisk",
                target_bootdisk_selectview(
                    &runinfo.disks,
                    advanced_options.clone(),
                    // At least one disk must always exist to even get to this point,
                    // see proxmox_installer_common::setup::installer_setup()
                    &options.disks[0],
                ),
            )
            .with_name("bootdisk-options-target-disk");

        let product_conf = siv
            .user_data::<InstallerState>()
            .map(|state| state.setup_info.config.clone())
            .unwrap(); // Safety: InstallerState must always be set

        let advanced_button = LinearLayout::horizontal()
            .child(DummyView.full_width())
            .child(Button::new("Advanced options", {
                let runinfo = runinfo.clone();
                let options = advanced_options.clone();
                move |siv| {
                    let mut view =
                        advanced_options_view(&runinfo, options.clone(), product_conf.clone());

                    // Pre-compute the child's layout, since it might depend on the size. Without this,
                    // the view will be empty until focused.
                    // The screen size never changes in our case, so this is completely OK.
                    view.layout(siv.screen_size());

                    siv.add_layer(view);
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
        // The simple disk selector, as well as the advanced bootdisk dialog save their
        // info on submit directly to the shared `BootdiskOptionsRef` - so just clone() + return
        // it.
        let options = self.advanced_options.lock().unwrap().clone();
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
    fn new(
        runinfo: &RuntimeInfo,
        options_ref: BootdiskOptionsRef,
        product_conf: ProductConfig,
    ) -> Self {
        let filter_btrfs =
            |fstype: &&FsType| -> bool { product_conf.enable_btrfs || !fstype.is_btrfs() };
        let options = options_ref.lock().unwrap();

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
                let options_ref = options_ref.clone();
                move |siv, fstype| {
                    Self::fstype_on_submit(siv, fstype, options_ref.clone());
                }
            });

        let mut view = LinearLayout::vertical()
            .child(DummyView.full_width())
            .child(FormView::new().child("Filesystem", fstype_select))
            .child(DummyView.full_width());

        // Create the appropriate (inner) advanced options view
        match &options.advanced {
            AdvancedBootdiskOptions::Lvm(lvm) => view.add_child(LvmBootdiskOptionsView::new(
                &options.disks[0],
                lvm,
                &product_conf,
            )),
            AdvancedBootdiskOptions::Zfs(zfs) => {
                view.add_child(ZfsBootdiskOptionsView::new(runinfo, zfs, &product_conf))
            }
            AdvancedBootdiskOptions::Btrfs(btrfs) => {
                view.add_child(BtrfsBootdiskOptionsView::new(runinfo, btrfs))
            }
        };

        Self { view }
    }

    /// Called when a new filesystem type is chosen by the user.
    /// It first creates the inner (filesystem-specific) options view according to the selected
    /// filesystem type.
    /// Further, it replaces the (outer) bootdisk selector in the main dialog, either with a
    /// selector for LVM configurations or a simple label displaying the chosen RAID for ZFS and
    /// Btrfs configurations.
    ///
    /// # Arguments
    /// * `siv` - Cursive instance
    /// * `fstype` - The chosen filesystem type by the user, for which the UI should be
    ///              updated accordingly
    /// * `options_ref` - [`BootdiskOptionsRef`] where advanced disk options should be saved to
    fn fstype_on_submit(siv: &mut Cursive, fstype: &FsType, options_ref: BootdiskOptionsRef) {
        let state = siv.user_data::<InstallerState>().unwrap();
        let runinfo = state.runtime_info.clone();
        let product_conf = state.setup_info.config.clone();

        // Only used for LVM configurations, ZFS and Btrfs do not use the target disk selector
        // Must be done here, as we cannot mutable borrow `siv` a second time inside the closure
        // below.
        let selected_lvm_disk = siv
            .find_name::<FormView>("bootdisk-options-target-disk")
            .and_then(|v| v.get_value::<SelectView<Disk>, _>(0))
            // If not defined, then the view was switched from a non-LVM filesystem to a LVM one.
            // Just use the first disk is such a case.
            .unwrap_or_else(|| runinfo.disks[0].clone());

        // Update the (inner) options view
        let screen_size = siv.screen_size();
        siv.call_on_name("advanced-bootdisk-options-dialog", |view: &mut Dialog| {
            if let Some(AdvancedBootdiskOptionsView { view }) =
                view.get_content_mut().downcast_mut()
            {
                view.remove_child(3);
                match fstype {
                    FsType::Ext4 | FsType::Xfs => {
                        view.add_child(LvmBootdiskOptionsView::new_with_defaults(
                            &selected_lvm_disk,
                            &product_conf,
                        ))
                    }
                    FsType::Zfs(_) => view.add_child(ZfsBootdiskOptionsView::new_with_defaults(
                        &runinfo,
                        &product_conf,
                    )),
                    FsType::Btrfs(_) => {
                        view.add_child(BtrfsBootdiskOptionsView::new_with_defaults(&runinfo))
                    }
                }

                // Pre-compute the child's layout, since it might depend on the size. Without this,
                // the view will be empty until focused.
                // The screen size never changes in our case, so this is completely OK.
                view.layout(screen_size);
            }
        });

        // The "bootdisk-options-target-disk" view might be either a `SelectView` (if ext4 of XFS
        // is used) or a label containing the filesytem/RAID type (for ZFS and Btrfs).
        // Now, unconditionally replace it with the appropriate type of these two, depending on the
        // newly selected filesystem type.
        siv.call_on_name(
            "bootdisk-options-target-disk",
            move |view: &mut FormView| match fstype {
                FsType::Ext4 | FsType::Xfs => {
                    view.replace_child(
                        0,
                        target_bootdisk_selectview(&runinfo.disks, options_ref, &selected_lvm_disk),
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
            let (disk, advanced) = view
                .get_values()
                .ok_or("Failed to retrieve advanced bootdisk options")?;

            Ok(BootdiskOptions {
                disks: vec![disk],
                fstype,
                advanced: AdvancedBootdiskOptions::Lvm(advanced),
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
    disk: Disk,
    has_extra_fields: bool,
}

impl LvmBootdiskOptionsView {
    fn new(disk: &Disk, options: &LvmBootdiskOptions, product_conf: &ProductConfig) -> Self {
        let show_extra_fields = product_conf.product == ProxmoxProduct::PVE;

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
            disk: disk.clone(),
            has_extra_fields: show_extra_fields,
        }
    }

    fn new_with_defaults(disk: &Disk, product_conf: &ProductConfig) -> Self {
        Self::new(disk, &LvmBootdiskOptions::defaults_from(disk), product_conf)
    }

    fn get_values(&mut self) -> Option<(Disk, LvmBootdiskOptions)> {
        let min_lvm_free_id = if self.has_extra_fields { 4 } else { 2 };

        let max_root_size = self
            .has_extra_fields
            .then(|| self.view.get_value::<DiskSizeEditView, _>(2))
            .flatten();
        let max_data_size = self
            .has_extra_fields
            .then(|| self.view.get_value::<DiskSizeEditView, _>(3))
            .flatten();

        Some((
            self.disk.clone(),
            LvmBootdiskOptions {
                total_size: self.view.get_value::<DiskSizeEditView, _>(0)?,
                swap_size: self.view.get_value::<DiskSizeEditView, _>(1),
                max_root_size,
                max_data_size,
                min_lvm_free: self.view.get_value::<DiskSizeEditView, _>(min_lvm_free_id),
            },
        ))
    }
}

impl ViewWrapper for LvmBootdiskOptionsView {
    cursive::wrap_impl!(self.view: FormView);
}

struct MultiDiskOptionsView<T> {
    view: LinearLayout,
    layout_data: Option<(Vec<Disk>, Vec<usize>, T)>,
    phantom: PhantomData<T>,
}

impl<T: View> MultiDiskOptionsView<T> {
    const DISK_FORM_VIEW_ID: &'static str = "multidisk-disk-form";

    fn new(avail_disks: &[Disk], selected_disks: &[usize], options_view: T) -> Self {
        Self {
            view: LinearLayout::vertical().child(DummyView).child(DummyView),
            layout_data: Some((avail_disks.to_vec(), selected_disks.to_vec(), options_view)),
            phantom: PhantomData,
        }
    }

    fn top_panel(mut self, view: impl View) -> Self {
        self.view.remove_child(0);
        self.view.insert_child(0, Panel::new(view));
        self
    }

    fn get_options_view(&mut self) -> Option<&T> {
        let inner = self.view.get_child(1)?;

        if let Some(view) = inner.downcast_ref::<LinearLayout>() {
            view.get_child(2)?
                .downcast_ref::<LinearLayout>()?
                .get_child(2)?
                .downcast_ref::<T>()
        } else if let Some(view) = inner.downcast_ref::<TabbedView>() {
            view.get(1)?
                .downcast_ref::<PaddedView<T>>()
                .map(|v| v.get_inner())
        } else {
            None
        }
    }

    fn get_disk_form(&mut self) -> Option<ViewRef<FormView>> {
        let inner = self.view.get_child_mut(1)?;

        let view = if let Some(view) = inner.downcast_mut::<LinearLayout>() {
            view.get_child_mut(0)?
                .downcast_mut::<LinearLayout>()?
                .get_child_mut(2)
        } else if let Some(view) = inner.downcast_mut::<TabbedView>() {
            view.get_mut(0)?
                .downcast_mut::<PaddedView<LinearLayout>>()
                .map(|v| v.get_inner_mut())?
                .get_child_mut(0)
        } else {
            None
        };

        view?
            .downcast_mut::<ScrollView<NamedView<FormView>>>()
            .map(ScrollView::get_inner_mut)
            .map(NamedView::get_mut)
    }

    /// This function returns a tuple of vectors. The first vector contains the currently selected
    /// disks in order of their selection slot. Empty slots are filtered out. The second vector
    /// contains indices of each slot's selection, which enables us to restore the selection even
    /// for empty slots.
    fn get_disks_and_selection(&mut self) -> Option<(Vec<Disk>, Vec<usize>)> {
        let mut disks = vec![];
        let mut selected_disks = Vec::new();
        let disk_form = self.get_disk_form()?;

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

    fn do_layout(&mut self, size: Vec2) {
        let Some((avail_disks, selected_disks, options_view)) = self.layout_data.take() else {
            panic!("cannot do layout without data!");
        };

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

        let mut disk_select_view = LinearLayout::vertical().child(ScrollView::new(
            disk_form.with_name(Self::DISK_FORM_VIEW_ID),
        ));

        if avail_disks.len() > 3 {
            let do_not_use_index = selectable_disks.len() - 1;
            let deselect_all_button = Button::new("Deselect all", move |siv| {
                siv.call_on_name(Self::DISK_FORM_VIEW_ID, |view: &mut FormView| {
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

        self.view.remove_child(1);

        if size.x > 80 {
            disk_select_view.insert_child(0, DummyView);
            disk_select_view.insert_child(0, TextView::new("Disk setup").center());

            let view = LinearLayout::horizontal()
                .child(disk_select_view)
                .child(DummyView.fixed_width(3))
                .child(
                    LinearLayout::vertical()
                        .child(TextView::new("Advanced options").center())
                        .child(DummyView)
                        .child(options_view),
                );

            self.view.add_child(view);
        } else {
            let view = TabbedView::new()
                .tab("Disk setup", PaddedView::lrtb(0, 0, 1, 0, disk_select_view))
                .tab(
                    "Advanced options",
                    PaddedView::lrtb(0, 0, 1, 0, options_view),
                );

            self.view.add_child(view);
        }
    }
}

impl<T: 'static + View> ViewWrapper for MultiDiskOptionsView<T> {
    cursive::wrap_impl!(self.view: LinearLayout);

    fn wrap_layout(&mut self, size: Vec2) {
        if self.layout_data.is_some() {
            self.do_layout(size);
        }

        self.view.layout(size)
    }

    fn wrap_needs_relayout(&self) -> bool {
        self.layout_data.is_some() || self.view.needs_relayout()
    }
}

struct BtrfsBootdiskOptionsView {
    view: MultiDiskOptionsView<FormView>,
}

impl BtrfsBootdiskOptionsView {
    fn new(runinfo: &RuntimeInfo, options: &BtrfsBootdiskOptions) -> Self {
        let inner = FormView::new()
            .child(
                "compress",
                SelectView::new()
                    .popup()
                    .with_all(BTRFS_COMPRESS_OPTIONS.iter().map(|o| (o.to_string(), *o)))
                    .selected(
                        BTRFS_COMPRESS_OPTIONS
                            .iter()
                            .position(|o| *o == options.compress)
                            .unwrap_or_default(),
                    ),
            )
            .child("hdsize", DiskSizeEditView::new().content(options.disk_size));

        let view = MultiDiskOptionsView::new(&runinfo.disks, &options.selected_disks, inner)
            .top_panel(TextView::new("Btrfs integration is a technology preview!").center());

        Self { view }
    }

    fn new_with_defaults(runinfo: &RuntimeInfo) -> Self {
        Self::new(
            runinfo,
            &BtrfsBootdiskOptions::defaults_from(&runinfo.disks),
        )
    }

    fn get_values(&mut self) -> Option<(Vec<Disk>, BtrfsBootdiskOptions)> {
        let (disks, selected_disks) = self.view.get_disks_and_selection()?;
        let view = self.view.get_options_view()?;
        let compress = view.get_value::<SelectView<_>, _>(0)?;
        let disk_size = view.get_value::<DiskSizeEditView, _>(1)?;

        Some((
            disks,
            BtrfsBootdiskOptions {
                disk_size,
                selected_disks,
                compress,
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
    fn new(
        runinfo: &RuntimeInfo,
        options: &ZfsBootdiskOptions,
        product_conf: &ProductConfig,
    ) -> Self {
        let arc_max_view = {
            let view = IntegerEditView::new_with_suffix("MiB").max_value(runinfo.total_memory);

            // For PVE "force" the default value, for other products place the recommended value
            // only in the placeholder. This causes for the latter to not write the module option
            // if the value is never modified by the user.
            if product_conf.product == ProxmoxProduct::PVE {
                view.content(options.arc_max)
            } else {
                let view = view.placeholder(runinfo.total_memory / 2);

                if options.arc_max != 0 {
                    view.content(options.arc_max)
                } else {
                    view
                }
            }
        };

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
            .child(
                "copies",
                IntegerEditView::new().content(options.copies).max_value(3),
            )
            .child("ARC max size", arc_max_view)
            .child("hdsize", DiskSizeEditView::new().content(options.disk_size));

        let view = MultiDiskOptionsView::new(&runinfo.disks, &options.selected_disks, inner)
            .top_panel(TextView::new(
                "ZFS is not compatible with hardware RAID controllers, for details see the documentation."
            ).center());

        Self { view }
    }

    fn new_with_defaults(runinfo: &RuntimeInfo, product_conf: &ProductConfig) -> Self {
        Self::new(
            runinfo,
            &ZfsBootdiskOptions::defaults_from(runinfo, product_conf),
            product_conf,
        )
    }

    fn get_values(&mut self) -> Option<(Vec<Disk>, ZfsBootdiskOptions)> {
        let (disks, selected_disks) = self.view.get_disks_and_selection()?;
        let view = self.view.get_options_view()?;

        let ashift = view.get_value::<IntegerEditView, _>(0)?;
        let compress = view.get_value::<SelectView<_>, _>(1)?;
        let checksum = view.get_value::<SelectView<_>, _>(2)?;
        let copies = view.get_value::<IntegerEditView, _>(3)?;
        let disk_size = view.get_value::<DiskSizeEditView, _>(5)?;

        // If a value is set, return that and clamp it to at least [`ZFS_ARC_MIN_SIZE_MIB`].
        //
        // Otherwise, if no value was set or an error occurred return `0`. The former simply means
        // that the placeholder value is still there.
        let arc_max = view
            .get_child::<IntegerEditView>(4)?
            .get_content_maybe()
            .map_or(Ok(0), |v| v.map(|v| v.max(ZFS_ARC_MIN_SIZE_MIB)))
            .unwrap_or(0);

        Some((
            disks,
            ZfsBootdiskOptions {
                ashift,
                compress,
                checksum,
                copies,
                arc_max,
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
    runinfo: &RuntimeInfo,
    options_ref: BootdiskOptionsRef,
    product_conf: ProductConfig,
) -> impl View {
    Dialog::around(AdvancedBootdiskOptionsView::new(
        runinfo,
        options_ref.clone(),
        product_conf,
    ))
    .title("Advanced bootdisk options")
    .button("Ok", {
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
            *options_ref.lock().unwrap() = options;
        }
    })
    .with_name("advanced-bootdisk-options-dialog")
    .max_size((120, 40))
}

/// Creates a select view for all disks specified.
///
/// # Arguments
///
/// * `avail_disks` - Disks that should be shown in the select view
/// * `options_ref` - [`BootdiskOptionsRef`] where advanced disk options should be saved to
/// * `selected_disk` - Optional, specifies which disk should be pre-selected
fn target_bootdisk_selectview(
    avail_disks: &[Disk],
    options_ref: BootdiskOptionsRef,
    selected_disk: &Disk,
) -> SelectView<Disk> {
    let selected_disk_pos = avail_disks
        .iter()
        .position(|d| d.index == selected_disk.index)
        .unwrap_or_default();

    SelectView::new()
        .popup()
        .with_all(avail_disks.iter().map(|d| (d.to_string(), d.clone())))
        .selected(selected_disk_pos)
        .on_submit(move |_, disk| {
            let mut options = options_ref.lock().unwrap();
            options.disks = vec![disk.clone()];
            options.advanced =
                AdvancedBootdiskOptions::Lvm(LvmBootdiskOptions::defaults_from(disk));
        })
}
