use std::{cell::RefCell, rc::Rc};

use super::{DiskSizeFormInputView, FormInputView, FormInputViewGetValue};
use crate::options::{
    AdvancedBootdiskOptions, BootdiskOptions, Disk, FsType, LvmBootdiskOptions, FS_TYPES,
};
use cursive::{
    view::{Nameable, Resizable, ViewWrapper},
    views::{Button, Dialog, DummyView, LinearLayout, SelectView},
    View,
};

pub struct BootdiskOptionsView {
    view: LinearLayout,
    advanced_options: Rc<RefCell<(FsType, AdvancedBootdiskOptions)>>,
}

impl BootdiskOptionsView {
    pub fn new(disks: &[Disk], options: &BootdiskOptions) -> Self {
        let bootdisk_select = FormInputView::new(
            "Target harddisk",
            SelectView::new()
                .popup()
                .with_all(disks.iter().map(|d| (d.to_string(), d.clone()))),
        );

        let advanced_options = Rc::new(RefCell::new((options.fstype, options.advanced.clone())));

        let advanced_button = LinearLayout::horizontal()
            .child(DummyView.full_width())
            .child(Button::new("Advanced options", {
                let options = advanced_options.clone();
                move |siv| {
                    siv.add_layer(advanced_options_view(options.clone()));
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
        let disk = self
            .view
            .get_child(0)?
            .downcast_ref::<FormInputView<SelectView<Disk>>>()?
            .get_value()?;

        let (fstype, advanced) = (*self.advanced_options).clone().into_inner();

        Some(BootdiskOptions {
            disks: vec![disk],
            fstype,
            advanced,
        })
    }
}

impl ViewWrapper for BootdiskOptionsView {
    cursive::wrap_impl!(self.view: LinearLayout);
}

struct AdvancedBootdiskOptionsView {
    view: LinearLayout,
}

impl AdvancedBootdiskOptionsView {
    fn new((fstype, advanced): &(FsType, AdvancedBootdiskOptions)) -> Self {
        let fstype_select = FormInputView::new(
            "Filesystem",
            SelectView::new()
                .popup()
                .with_all(FS_TYPES.iter().map(|t| (t.to_string(), *t)))
                .selected(
                    FS_TYPES
                        .iter()
                        .position(|t| t == fstype)
                        .unwrap_or_default(),
                ),
        );

        let AdvancedBootdiskOptions::Lvm(lvm) = &advanced;

        let view = LinearLayout::vertical()
            .child(DummyView.full_width())
            .child(fstype_select)
            .child(DummyView.full_width())
            .child(LvmBootdiskOptionsView::new(lvm));

        Self { view }
    }

    fn get_values(&mut self) -> Option<(FsType, AdvancedBootdiskOptions)> {
        let fstype = self
            .view
            .get_child(1)?
            .downcast_ref::<FormInputView<SelectView<FsType>>>()?
            .get_value()?;

        let advanced = self
            .view
            .get_child_mut(3)?
            .downcast_mut::<LvmBootdiskOptionsView>()?
            .get_values()
            .map(AdvancedBootdiskOptions::Lvm)?;

        Some((fstype, advanced))
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

fn advanced_options_view(options: Rc<RefCell<(FsType, AdvancedBootdiskOptions)>>) -> impl View {
    Dialog::around(AdvancedBootdiskOptionsView::new(&(*options).borrow()))
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
                if let Some((fstype, advanced)) = options {
                    (*options_ref).borrow_mut().0 = fstype;
                    (*options_ref).borrow_mut().1 = advanced;
                }
            }
        })
        .with_name("advanced-bootdisk-options-dialog")
}
