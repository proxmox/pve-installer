use super::{DiskSizeFormInputView, FormInputView};
use crate::options::{
    AdvancedBootdiskOptions, BootdiskOptions, Disk, FsType, LvmBootdiskOptions, FS_TYPES,
};
use cursive::{
    view::{Finder, Nameable, Resizable, ViewWrapper},
    views::{DummyView, LinearLayout, SelectView},
};

pub struct BootdiskDialogView {
    view: LinearLayout,
}

impl BootdiskDialogView {
    pub fn new(options: &BootdiskOptions) -> Self {
        let AdvancedBootdiskOptions::Lvm(advanced) = &options.advanced;

        let fstype_select = FormInputView::new(
            "Filesystem",
            SelectView::new()
                .popup()
                .with_all(FS_TYPES.iter().map(|t| (t.to_string(), t)))
                .selected(
                    FS_TYPES
                        .iter()
                        .position(|t| *t == options.fstype)
                        .unwrap_or_default(),
                )
                .on_submit({
                    let disks = options.disks.clone();
                    let advanced = advanced.clone();
                    move |siv, fstype: &FsType| {
                        let view = match fstype {
                            FsType::Ext4 | FsType::Xfs => {
                                LvmBootdiskOptionsView::new(&disks, &advanced)
                            }
                        };

                        siv.call_on_name("bootdisk-options", |v: &mut LinearLayout| {
                            v.clear();
                            v.add_child(view);
                        });
                    }
                })
                .with_name("fstype")
                .full_width(),
        );

        let view = LinearLayout::vertical()
            .child(fstype_select)
            .child(DummyView)
            .child(LvmBootdiskOptionsView::new(&options.disks, advanced));

        Self { view }
    }

    pub fn get_values(&mut self) -> Option<AdvancedBootdiskOptions> {
        self.view
            .get_child_mut(2)?
            .downcast_mut::<LvmBootdiskOptionsView>()?
            .get_values()
            .map(AdvancedBootdiskOptions::Lvm)
    }
}

impl ViewWrapper for BootdiskDialogView {
    cursive::wrap_impl!(self.view: LinearLayout);
}

struct LvmBootdiskOptionsView {
    view: LinearLayout,
}

impl LvmBootdiskOptionsView {
    fn new(disks: &[Disk], options: &LvmBootdiskOptions) -> Self {
        let view = LinearLayout::vertical()
            .child(FormInputView::new(
                "Target harddisk",
                SelectView::new()
                    .popup()
                    .with_all(disks.iter().map(|d| (d.to_string(), d.clone())))
                    .with_name("bootdisk-disk"),
            ))
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
        let disk = self
            .view
            .call_on_name("bootdisk-disk", |view: &mut SelectView<Disk>| {
                view.selection()
            })?
            .map(|d| (*d).clone())?;

        let mut get_disksize_value = |i| {
            self.view
                .get_child_mut(i)?
                .downcast_mut::<DiskSizeFormInputView>()?
                .get_content()
        };

        Some(LvmBootdiskOptions {
            disk,
            total_size: get_disksize_value(1)?,
            swap_size: get_disksize_value(2)?,
            max_root_size: get_disksize_value(3)?,
            max_data_size: get_disksize_value(4)?,
            min_lvm_free: get_disksize_value(5)?,
        })
    }
}

impl ViewWrapper for LvmBootdiskOptionsView {
    cursive::wrap_impl!(self.view: LinearLayout);
}
