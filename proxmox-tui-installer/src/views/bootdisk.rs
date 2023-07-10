use std::{cell::RefCell, marker::PhantomData, rc::Rc};

use cursive::{
    view::{Nameable, Resizable, ViewWrapper},
    views::{
        Button, Dialog, DummyView, LinearLayout, NamedView, Panel, ScrollView, SelectView, TextView,
    },
    Cursive, View,
};

use super::{DiskSizeEditView, FormView, IntegerEditView};
use crate::options::{
    AdvancedBootdiskOptions, BootdiskOptions, BtrfsBootdiskOptions, Disk, FsType,
    LvmBootdiskOptions, ZfsBootdiskOptions, FS_TYPES, ZFS_CHECKSUM_OPTIONS, ZFS_COMPRESS_OPTIONS,
};
use crate::setup::ProxmoxProduct;

pub struct BootdiskOptionsView {
    view: LinearLayout,
    advanced_options: Rc<RefCell<BootdiskOptions>>,
}

impl BootdiskOptionsView {
    pub fn new(disks: &[Disk], options: &BootdiskOptions) -> Self {
        let bootdisk_form = FormView::new()
            .child(
                "Target harddisk",
                SelectView::new()
                    .popup()
                    .with_all(disks.iter().map(|d| (d.to_string(), d.clone()))),
            )
            .with_name("bootdisk-options-target-disk");

        let advanced_options = Rc::new(RefCell::new(options.clone()));

        let advanced_button = LinearLayout::horizontal()
            .child(DummyView.full_width())
            .child(Button::new("Advanced options", {
                let disks = disks.to_owned();
                let options = advanced_options.clone();
                move |siv| {
                    siv.add_layer(advanced_options_view(&disks, options.clone()));
                }
            }));

        let view = LinearLayout::vertical()
            .child(bootdisk_form)
            .child(DummyView)
            .child(advanced_button);

        Self {
            view,
            advanced_options,
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
                .ok_or("failed to retrieve filesystem type")?;

            options.disks = vec![disk];
        }

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
    fn new(disks: &[Disk], options: &BootdiskOptions) -> Self {
        let enable_btrfs = crate::setup_info().config.enable_btrfs;

        let filter_btrfs = |fstype: &&FsType| -> bool { enable_btrfs || !fstype.is_btrfs() };

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
            AdvancedBootdiskOptions::Lvm(lvm) => view.add_child(LvmBootdiskOptionsView::new(lvm)),
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
        siv.call_on_name("advanced-bootdisk-options-dialog", |view: &mut Dialog| {
            if let Some(AdvancedBootdiskOptionsView { view }) =
                view.get_content_mut().downcast_mut()
            {
                view.remove_child(3);
                match fstype {
                    FsType::Ext4 | FsType::Xfs => view.add_child(LvmBootdiskOptionsView::new(
                        &LvmBootdiskOptions::defaults_from(&disks[0]),
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

    fn get_values(&mut self) -> Option<BootdiskOptions> {
        let fstype = self
            .view
            .get_child(1)?
            .downcast_ref::<FormView>()?
            .get_value::<SelectView<FsType>, _>(0)?;

        let advanced = self.view.get_child_mut(3)?;

        if let Some(view) = advanced.downcast_mut::<LvmBootdiskOptionsView>() {
            Some(BootdiskOptions {
                disks: vec![],
                fstype,
                advanced: view.get_values().map(AdvancedBootdiskOptions::Lvm)?,
            })
        } else if let Some(view) = advanced.downcast_mut::<ZfsBootdiskOptionsView>() {
            let (disks, advanced) = view.get_values()?;

            Some(BootdiskOptions {
                disks,
                fstype,
                advanced: AdvancedBootdiskOptions::Zfs(advanced),
            })
        } else if let Some(view) = advanced.downcast_mut::<BtrfsBootdiskOptionsView>() {
            let (disks, advanced) = view.get_values()?;

            Some(BootdiskOptions {
                disks,
                fstype,
                advanced: AdvancedBootdiskOptions::Btrfs(advanced),
            })
        } else {
            None
        }
    }
}

impl ViewWrapper for AdvancedBootdiskOptionsView {
    cursive::wrap_impl!(self.view: LinearLayout);
}

struct LvmBootdiskOptionsView {
    view: FormView,
}

impl LvmBootdiskOptionsView {
    fn new(options: &LvmBootdiskOptions) -> Self {
        let is_pve = crate::setup_info().config.product == ProxmoxProduct::PVE;
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
                is_pve,
                "Maximum root volume size",
                DiskSizeEditView::new_emptyable().content_maybe(options.max_root_size),
            )
            .child_conditional(
                is_pve,
                "Maximum data volume size",
                DiskSizeEditView::new_emptyable().content_maybe(options.max_data_size),
            )
            .child(
                "Minimum free LVM space",
                DiskSizeEditView::new_emptyable().content_maybe(options.min_lvm_free),
            );

        Self { view }
    }

    fn get_values(&mut self) -> Option<LvmBootdiskOptions> {
        let is_pve = crate::setup_info().config.product == ProxmoxProduct::PVE;
        let min_lvm_free_id = if is_pve { 4 } else { 2 };
        let max_root_size = if is_pve {
            self.view.get_value::<DiskSizeEditView, _>(2)
        } else {
            None
        };
        let max_data_size = if is_pve {
            self.view.get_value::<DiskSizeEditView, _>(3)
        } else {
            None
        };
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

        let disk_select_view = LinearLayout::vertical()
            .child(TextView::new("Disk setup").center())
            .child(DummyView)
            .child(ScrollView::new(disk_form));

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
            .get_child(view_top_index)?
            .downcast_ref::<LinearLayout>()?
            .get_child(0)?
            .downcast_ref::<LinearLayout>()?
            .get_child(2)?
            .downcast_ref::<ScrollView<FormView>>()?
            .get_inner();

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

fn advanced_options_view(disks: &[Disk], options: Rc<RefCell<BootdiskOptions>>) -> impl View {
    Dialog::around(AdvancedBootdiskOptionsView::new(
        disks,
        &(*options).borrow(),
    ))
    .title("Advanced bootdisk options")
    .button("Ok", {
        let options_ref = options.clone();
        move |siv| {
            let options = siv
                .call_on_name("advanced-bootdisk-options-dialog", |view: &mut Dialog| {
                    view.get_content_mut()
                        .downcast_mut()
                        .and_then(AdvancedBootdiskOptionsView::get_values)
                })
                .flatten();

            if let Some(disks) = options.as_ref().map(|opts| &opts.disks) {
                if disks.len() > 1 {
                    for i in 0..(disks.len() - 1) {
                        let check_disk = &disks[i];
                        for disk in &disks[(i + 1)..] {
                            if disk.index == check_disk.index {
                                siv.add_layer(Dialog::info(format!(
                                    "cannot select same disk ({}) twice",
                                    disk.path
                                )));
                                return;
                            }
                        }
                    }
                }
            }

            siv.pop_layer();
            if let Some(options) = options {
                *(*options_ref).borrow_mut() = options;
            }
        }
    })
    .with_name("advanced-bootdisk-options-dialog")
    .max_size((120, 40))
}
