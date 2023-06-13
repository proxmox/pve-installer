use std::{cell::RefCell, rc::Rc};

use super::{DiskSizeFormInputView, FormInputView, FormInputViewGetValue, IntegerEditView};
use crate::options::{
    AdvancedBootdiskOptions, BootdiskOptions, Disk, FsType, LvmBootdiskOptions, ZfsBootdiskOptions,
    ZfsChecksumOption, ZfsCompressOption, FS_TYPES, ZFS_CHECKSUM_OPTIONS, ZFS_COMPRESS_OPTIONS,
};
use cursive::{
    theme::Effect,
    view::{Nameable, Resizable, ViewWrapper},
    views::{Button, Dialog, DummyView, LinearLayout, NamedView, SelectView, TextView},
    Cursive, View,
};

pub struct BootdiskOptionsView {
    view: LinearLayout,
    advanced_options: Rc<RefCell<BootdiskOptions>>,
}

impl BootdiskOptionsView {
    pub fn new(disks: &[Disk], options: &BootdiskOptions) -> Self {
        let bootdisk_select = FormInputView::new(
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
            .child(bootdisk_select)
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
                .downcast_mut::<NamedView<FormInputView<SelectView<Disk>>>>()?
                .get_mut()
                .get_value()?;

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
        let fstype_select = FormInputView::new(
            "Filesystem",
            SelectView::new()
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
                }),
        );

        let mut view = LinearLayout::vertical()
            .child(DummyView.full_width())
            .child(fstype_select)
            .child(DummyView.full_width());

        match &options.advanced {
            AdvancedBootdiskOptions::Lvm(lvm) => view.add_child(LvmBootdiskOptionsView::new(lvm)),
            AdvancedBootdiskOptions::Zfs(zfs) => {
                view.add_child(ZfsBootdiskOptionsView::new(disks, zfs))
            }
        };

        Self { view }
    }

    fn fstype_on_submit(siv: &mut Cursive, disks: &[Disk], fstype: &FsType) {
        siv.call_on_name("advanced-bootdisk-options-dialog", |view: &mut Dialog| {
            if let Some(AdvancedBootdiskOptionsView { view }) =
                view.get_content_mut()
                    .downcast_mut::<AdvancedBootdiskOptionsView>()
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
                }
            }
        });

        siv.call_on_name(
            "bootdisk-options-target-disk",
            |view: &mut FormInputView<SelectView<Disk>>| match fstype {
                FsType::Ext4 | FsType::Xfs => view.replace_inner(
                    SelectView::new()
                        .popup()
                        .with_all(disks.iter().map(|d| (d.to_string(), d.clone()))),
                ),
                other => view.replace_inner(TextView::new(other.to_string())),
            },
        );
    }

    fn get_values(&mut self) -> Option<BootdiskOptions> {
        let fstype = self
            .view
            .get_child(1)?
            .downcast_ref::<FormInputView<SelectView<FsType>>>()?
            .get_value()?;

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
        } else {
            None
        }
    }
}

impl ViewWrapper for AdvancedBootdiskOptionsView {
    cursive::wrap_impl!(self.view: LinearLayout);
}

struct LvmBootdiskOptionsView {
    view: LinearLayout,
}

impl LvmBootdiskOptionsView {
    fn new(options: &LvmBootdiskOptions) -> Self {
        // TODO: Set maximum accordingly to disk size
        let view = LinearLayout::vertical()
            .child(DiskSizeFormInputView::new("Total size").content(options.total_size))
            .child(DiskSizeFormInputView::new("Swap size").content(options.swap_size))
            .child(
                DiskSizeFormInputView::new("Maximum root volume size")
                    .content(options.max_root_size),
            )
            .child(
                DiskSizeFormInputView::new("Maximum data volume size")
                    .content(options.max_data_size),
            )
            .child(
                DiskSizeFormInputView::new("Minimum free LVM space").content(options.min_lvm_free),
            );

        Self { view }
    }

    fn get_values(&mut self) -> Option<LvmBootdiskOptions> {
        let mut get_disksize_value = |i| {
            self.view
                .get_child_mut(i)?
                .downcast_mut::<DiskSizeFormInputView>()?
                .get_content()
        };

        Some(LvmBootdiskOptions {
            total_size: get_disksize_value(0)?,
            swap_size: get_disksize_value(1)?,
            max_root_size: get_disksize_value(2)?,
            max_data_size: get_disksize_value(3)?,
            min_lvm_free: get_disksize_value(4)?,
        })
    }
}

impl ViewWrapper for LvmBootdiskOptionsView {
    cursive::wrap_impl!(self.view: LinearLayout);
}

struct ZfsBootdiskOptionsView {
    view: LinearLayout,
}

impl ZfsBootdiskOptionsView {
    // TODO: Re-apply previous disk selection from `options` correctly
    fn new(disks: &[Disk], options: &ZfsBootdiskOptions) -> Self {
        let mut disk_select_view = LinearLayout::vertical()
            .child(
                TextView::new("Disk setup")
                    .center()
                    .style(Effect::Underline),
            )
            .child(DummyView);

        for i in 0..disks.len() {
            disk_select_view.add_child(FormInputView::new(
                &format!("Harddisk {i}"),
                SelectView::new()
                    .popup()
                    .with_all(disks.iter().map(|d| (d.to_string(), d.clone())))
                    .selected(i),
            ));
        }

        let options_view = LinearLayout::vertical()
            .child(
                TextView::new("Advanced options")
                    .center()
                    .style(Effect::Underline),
            )
            .child(DummyView)
            .child(FormInputView::new(
                "ashift",
                IntegerEditView::new().content(options.ashift),
            ))
            .child(FormInputView::new(
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
            ))
            .child(FormInputView::new(
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
            ))
            .child(FormInputView::new(
                "copies",
                IntegerEditView::new().content(options.copies),
            ))
            .child(DiskSizeFormInputView::new("hdsize").content(options.disk_size));

        let view = LinearLayout::horizontal()
            .child(disk_select_view)
            .child(DummyView.fixed_width(3))
            .child(options_view);

        Self { view }
    }

    fn get_values(&mut self) -> Option<(Vec<Disk>, ZfsBootdiskOptions)> {
        let mut disks = vec![];

        let disk_select_view = self.view.get_child(0)?.downcast_ref::<LinearLayout>()?;

        for i in 2..disk_select_view.len() {
            let disk = disk_select_view
                .get_child(i)?
                .downcast_ref::<FormInputView<SelectView<Disk>>>()?
                .get_value()?;

            disks.push(disk);
        }

        let options_view = self.view.get_child_mut(2)?.downcast_mut::<LinearLayout>()?;

        let ashift = options_view
            .get_child(2)?
            .downcast_ref::<FormInputView<IntegerEditView>>()?
            .get_value()?;

        let compress = options_view
            .get_child(3)?
            .downcast_ref::<FormInputView<SelectView<ZfsCompressOption>>>()?
            .get_value()?;

        let checksum = options_view
            .get_child(4)?
            .downcast_ref::<FormInputView<SelectView<ZfsChecksumOption>>>()?
            .get_value()?;

        let copies = options_view
            .get_child(5)?
            .downcast_ref::<FormInputView<IntegerEditView>>()?
            .get_value()?;

        let disk_size = options_view
            .get_child_mut(6)?
            .downcast_mut::<DiskSizeFormInputView>()?
            .get_content()?;

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
    cursive::wrap_impl!(self.view: LinearLayout);
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
                        .downcast_mut::<AdvancedBootdiskOptionsView>()
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
