use super::{DiskSizeEditView, FormView, IntegerEditView};
use crate::options::{
    AdvancedBootdiskOptions, BootdiskOptions, BtrfsBootdiskOptions, Disk, FsType,
    LvmBootdiskOptions, ZfsBootdiskOptions, FS_TYPES, ZFS_CHECKSUM_OPTIONS, ZFS_COMPRESS_OPTIONS,
};
use cursive::{
    theme::Effect,
    view::{Nameable, Resizable, ViewWrapper},
    views::{Button, Dialog, DummyView, LinearLayout, NamedView, SelectView, TextView},
    Cursive, View,
};
use std::{cell::RefCell, marker::PhantomData, rc::Rc};

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

    pub fn get_values(&mut self) -> Option<BootdiskOptions> {
        let mut options = (*self.advanced_options).clone().into_inner();

        if [FsType::Ext4, FsType::Xfs].contains(&options.fstype) {
            let disk = self
                .view
                .get_child_mut(0)?
                .downcast_mut::<NamedView<FormView>>()?
                .get_mut()
                .get_value::<SelectView<Disk>, _>(0)?;

            options.disks = vec![disk];
        }

        Some(options)
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
        let fstype_select = SelectView::new()
            .popup()
            .with_all(FS_TYPES.iter().map(|t| (t.to_string(), *t)))
            .selected(
                FS_TYPES
                    .iter()
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
                        &ZfsBootdiskOptions::defaults_from(&disks[0]),
                    )),
                    FsType::Btrfs(_) => view.add_child(BtrfsBootdiskOptionsView::new(
                        disks,
                        &BtrfsBootdiskOptions::defaults_from(&disks[0]),
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
        // TODO: Set maximum accordingly to disk size
        let view = FormView::new()
            .child(
                "Total size",
                DiskSizeEditView::new().content(options.total_size),
            )
            .child(
                "Swap size",
                DiskSizeEditView::new().content(options.swap_size),
            )
            .child(
                "Maximum root volume size",
                DiskSizeEditView::new().content(options.max_root_size),
            )
            .child(
                "Maximum data volume size",
                DiskSizeEditView::new().content(options.max_data_size),
            )
            .child(
                "Minimum free LVM space",
                DiskSizeEditView::new().content(options.min_lvm_free),
            );

        Self { view }
    }

    fn get_values(&mut self) -> Option<LvmBootdiskOptions> {
        Some(LvmBootdiskOptions {
            total_size: self.view.get_value::<DiskSizeEditView, _>(0)?,
            swap_size: self.view.get_value::<DiskSizeEditView, _>(1)?,
            max_root_size: self.view.get_value::<DiskSizeEditView, _>(2)?,
            max_data_size: self.view.get_value::<DiskSizeEditView, _>(3)?,
            min_lvm_free: self.view.get_value::<DiskSizeEditView, _>(4)?,
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
    fn new(avail_disks: &[Disk], options_view: T) -> Self {
        let mut disk_form = FormView::new();
        for i in 0..avail_disks.len() {
            disk_form.add_child(
                &format!("Harddisk {i}"),
                SelectView::new()
                    .popup()
                    .with_all(avail_disks.iter().map(|d| (d.to_string(), d.clone())))
                    .selected(i),
            );
        }

        let disk_select_view = LinearLayout::vertical()
            .child(
                TextView::new("Disk setup")
                    .center()
                    .style(Effect::Underline),
            )
            .child(DummyView)
            .child(disk_form);

        let options_view = LinearLayout::vertical()
            .child(
                TextView::new("Advanced options")
                    .center()
                    .style(Effect::Underline),
            )
            .child(DummyView)
            .child(options_view);

        let view = LinearLayout::horizontal()
            .child(disk_select_view)
            .child(DummyView.fixed_width(3))
            .child(options_view);

        Self {
            view,
            phantom: PhantomData,
        }
    }

    fn get_disks(&mut self) -> Option<Vec<Disk>> {
        let mut disks = vec![];
        let disk_form = self
            .view
            .get_child(0)?
            .downcast_ref::<LinearLayout>()?
            .get_child(2)?
            .downcast_ref::<FormView>()?;

        for i in 0..disk_form.len() {
            let disk = disk_form.get_value::<SelectView<Disk>, _>(i)?;

            disks.push(disk);
        }

        Some(disks)
    }

    fn inner_mut(&mut self) -> Option<&mut T> {
        self.view
            .get_child_mut(2)?
            .downcast_mut::<LinearLayout>()?
            .get_child_mut(2)?
            .downcast_mut::<T>()
    }
}

impl<T: 'static> ViewWrapper for MultiDiskOptionsView<T> {
    cursive::wrap_impl!(self.view: LinearLayout);
}

struct BtrfsBootdiskOptionsView {
    view: MultiDiskOptionsView<FormView>,
}

impl BtrfsBootdiskOptionsView {
    // TODO: Re-apply previous disk selection from `options` correctly
    fn new(disks: &[Disk], options: &BtrfsBootdiskOptions) -> Self {
        let view = MultiDiskOptionsView::new(
            disks,
            FormView::new().child("hdsize", DiskSizeEditView::new().content(options.disk_size)),
        );

        Self { view }
    }

    fn get_values(&mut self) -> Option<(Vec<Disk>, BtrfsBootdiskOptions)> {
        let disks = self.view.get_disks()?;
        let disk_size = self.view.inner_mut()?.get_value::<DiskSizeEditView, _>(0)?;

        Some((disks, BtrfsBootdiskOptions { disk_size }))
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

        Self {
            view: MultiDiskOptionsView::new(disks, inner),
        }
    }

    fn get_values(&mut self) -> Option<(Vec<Disk>, ZfsBootdiskOptions)> {
        let disks = self.view.get_disks()?;
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

            siv.pop_layer();
            if let Some(options) = options {
                *(*options_ref).borrow_mut() = options;
            }
        }
    })
    .with_name("advanced-bootdisk-options-dialog")
}
