#![forbid(unsafe_code)]

use cursive::{
    event::Event,
    view::{Resizable, ViewWrapper},
    views::{
        Button, Dialog, DummyView, LinearLayout, PaddedView, Panel, ResizedView, ScrollView,
        TextView,
    },
    Cursive, View,
};
use std::fmt;

// TextView::center() seems to garble the first two lines, so fix it manually here.
const LOGO: &str = r#"
       ____                                          _    __ _____
      / __ \_________  _  ______ ___  ____  _  __   | |  / / ____/
  / /_/ / ___/ __ \| |/_/ __ `__ \/ __ \| |/_/   | | / / __/
 / ____/ /  / /_/ />  </ / / / / / /_/ />  <     | |/ / /___
/_/   /_/   \____/_/|_/_/ /_/ /_/\____/_/|_|     |___/_____/
"#;

const TITLE: &str = "Proxmox VE Installer";

struct InstallerView {
    view: ResizedView<LinearLayout>,
}

impl InstallerView {
    pub fn new<T: View>(view: T) -> Self {
        let inner = LinearLayout::vertical()
            .child(PaddedView::lrtb(1, 1, 0, 1, TextView::new(LOGO).center()))
            .child(Dialog::around(view).title(TITLE));

        Self {
            // Limit the maximum to something reasonable, such that it won't get spread out much
            // depending on the screen.
            view: ResizedView::with_max_size((120, 40), inner),
        }
    }
}

impl ViewWrapper for InstallerView {
    cursive::wrap_impl!(self.view: ResizedView<LinearLayout>);
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
enum FsType {
    #[default]
    Ext4,
    Xfs,
}

impl fmt::Display for FsType {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let s = match self {
            FsType::Ext4 => "ext4",
            FsType::Xfs => "XFS",
        };
        write!(f, "{s}")
    }
}

#[derive(Clone, Debug)]
struct LvmBootdiskOptions {
    disk: Disk,
    total_size: u64,
    swap_size: u64,
    max_root_size: u64,
    max_data_size: u64,
    min_lvm_free: u64,
}

impl LvmBootdiskOptions {
    fn defaults_from(disk: &Disk) -> Self {
        let min_lvm_free = if disk.size > 128 * 1024 * 1024 {
            16 * 1024 * 1024
        } else {
            disk.size / 8
        };

        Self {
            disk: disk.clone(),
            total_size: disk.size,
            swap_size: 4 * 1024 * 1024, // TODO: value from installed memory
            max_root_size: 0,
            max_data_size: 0,
            min_lvm_free,
        }
    }
}

#[derive(Clone, Debug)]
enum AdvancedBootdiskOptions {
    Lvm(LvmBootdiskOptions),
}

#[derive(Clone, Debug)]
struct Disk {
    path: String,
    size: u64,
}

impl fmt::Display for Disk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // TODO: Format sizes properly with `proxmox-human-byte` once merged
        // https://lists.proxmox.com/pipermail/pbs-devel/2023-May/006125.html
        write!(f, "{} ({} B)", self.path, self.size)
    }
}

#[derive(Clone, Debug)]
struct BootdiskOptions {
    disks: Vec<Disk>,
    fstype: FsType,
    advanced: AdvancedBootdiskOptions,
}

#[derive(Clone, Debug)]
struct InstallerOptions {
    bootdisk: BootdiskOptions,
}

fn main() {
    let mut siv = cursive::termion();

    siv.clear_global_callbacks(Event::CtrlChar('c'));
    siv.set_on_pre_event(Event::CtrlChar('c'), trigger_abort_install_dialog);

    let disks = vec![Disk {
        path: "/dev/vda".to_owned(),
        size: 17179869184,
    }];
    siv.set_user_data(InstallerOptions {
        bootdisk: BootdiskOptions {
            disks: disks.clone(),
            fstype: FsType::default(),
            advanced: AdvancedBootdiskOptions::Lvm(LvmBootdiskOptions::defaults_from(&disks[0])),
        },
    });

    siv.add_active_screen();
    siv.screen_mut().add_layer(license_dialog());
    siv.run();
}

fn add_next_screen(
    constructor: &dyn Fn(&mut Cursive) -> InstallerView,
) -> Box<dyn Fn(&mut Cursive) + '_> {
    Box::new(|siv: &mut Cursive| {
        let v = constructor(siv);
        siv.add_active_screen();
        siv.screen_mut().add_layer(v);
    })
}

fn yes_no_dialog(
    siv: &mut Cursive,
    title: &str,
    text: &str,
    callback: &'static dyn Fn(&mut Cursive),
) {
    siv.add_layer(
        Dialog::around(TextView::new(text))
            .title(title)
            .dismiss_button("No")
            .button("Yes", callback),
    )
}

fn trigger_abort_install_dialog(siv: &mut Cursive) {
    #[cfg(debug_assertions)]
    siv.quit();

    #[cfg(not(debug_assertions))]
    yes_no_dialog(
        siv,
        "Abort installation?",
        "Are you sure you want to abort the installation?",
        &Cursive::quit,
    )
}

fn abort_install_button() -> Button {
    Button::new("Abort", trigger_abort_install_dialog)
}

fn get_eula() -> String {
    // TODO: properly using info from Proxmox::Install::Env::setup()
    std::fs::read_to_string("/cdrom/EULA")
        .unwrap_or_else(|_| "< Debug build - ignoring non-existing EULA >".to_owned())
}

fn license_dialog() -> InstallerView {
    let inner = LinearLayout::vertical()
        .child(PaddedView::lrtb(
            0,
            0,
            1,
            0,
            TextView::new("END USER LICENSE AGREEMENT (EULA)").center(),
        ))
        .child(Panel::new(ScrollView::new(
            TextView::new(get_eula()).center(),
        )))
        .child(PaddedView::lrtb(
            1,
            1,
            0,
            0,
            LinearLayout::horizontal()
                .child(abort_install_button())
                .child(DummyView.full_width())
                .child(Button::new("I agree", add_next_screen(&bootdisk_dialog))),
        ));

    InstallerView::new(inner)
}

fn bootdisk_dialog(siv: &mut Cursive) -> InstallerView {
    InstallerView::new(DummyView)
}
