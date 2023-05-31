#![forbid(unsafe_code)]

mod views;

use crate::views::DiskSizeFormInputView;
use cursive::{
    event::Event,
    view::{Finder, Nameable, Resizable, ViewWrapper},
    views::{
        Button, Dialog, DummyView, EditView, LinearLayout, PaddedView, Panel, ResizedView,
        ScrollView, SelectView, TextView,
    },
    Cursive, View,
};
use std::{
    fmt,
    net::{IpAddr, Ipv4Addr},
};
use views::{FormInputView, NumericEditView};

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
    pub fn new<T: View>(view: T, next_cb: Box<dyn Fn(&mut Cursive)>) -> Self {
        let inner = LinearLayout::vertical().child(view).child(PaddedView::lrtb(
            1,
            1,
            1,
            0,
            LinearLayout::horizontal()
                .child(abort_install_button())
                .child(DummyView.full_width())
                .child(Button::new("Previous", switch_to_prev_screen))
                .child(DummyView)
                .child(Button::new("Next", next_cb)),
        ));

        Self::with_raw(inner)
    }

    pub fn with_raw<T: View>(view: T) -> Self {
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

const FS_TYPES: &[FsType] = &[FsType::Ext4, FsType::Xfs];

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
struct TimezoneOptions {
    timezone: String,
    kb_layout: String,
}

impl Default for TimezoneOptions {
    fn default() -> Self {
        Self {
            timezone: "Europe/Vienna".to_owned(),
            kb_layout: "en_US".to_owned(),
        }
    }
}

#[derive(Clone, Debug)]
struct PasswordOptions {
    email: String,
    root_password: String,
}

impl Default for PasswordOptions {
    fn default() -> Self {
        Self {
            email: "mail@example.invalid".to_owned(),
            root_password: String::new(),
        }
    }
}

#[derive(Clone, Debug)]
struct NetworkOptions {
    ifname: String,
    fqdn: String,
    ip_addr: IpAddr,
    cidr_mask: usize,
    gateway: IpAddr,
    dns_server: IpAddr,
}

impl Default for NetworkOptions {
    fn default() -> Self {
        // TODO: Retrieve automatically
        Self {
            ifname: String::new(),
            fqdn: "pve.example.invalid".to_owned(),
            ip_addr: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            cidr_mask: 0,
            gateway: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            dns_server: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        }
    }
}

#[derive(Clone, Debug)]
struct InstallerOptions {
    bootdisk: BootdiskOptions,
    timezone: TimezoneOptions,
    password: PasswordOptions,
    network: NetworkOptions,
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
        timezone: TimezoneOptions::default(),
        password: PasswordOptions::default(),
        network: NetworkOptions::default(),
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

fn switch_to_prev_screen(siv: &mut Cursive) {
    let id = siv.active_screen().saturating_sub(1);
    siv.set_screen(id);
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
            1,
            0,
            LinearLayout::horizontal()
                .child(abort_install_button())
                .child(DummyView.full_width())
                .child(Button::new("I agree", add_next_screen(&bootdisk_dialog))),
        ));

    InstallerView::with_raw(inner)
}

fn bootdisk_dialog(siv: &mut Cursive) -> InstallerView {
    let options = siv
        .user_data::<InstallerOptions>()
        .map(|o| o.clone())
        .unwrap()
        .bootdisk;

    let AdvancedBootdiskOptions::Lvm(advanced) = options.advanced;

    let fstype_select = LinearLayout::horizontal()
        .child(TextView::new("Filesystem: "))
        .child(DummyView.full_width())
        .child(
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

    let inner = LinearLayout::vertical()
        .child(fstype_select)
        .child(DummyView)
        .child(
            LinearLayout::horizontal()
                .child(LvmBootdiskOptionsView::new(&options.disks, &advanced))
                .with_name("bootdisk-options"),
        );

    InstallerView::new(
        inner,
        Box::new(|siv| {
            let options = siv
                .call_on_name("bootdisk-options", |v: &mut LinearLayout| {
                    v.get_child_mut(0)?
                        .downcast_mut::<LvmBootdiskOptionsView>()?
                        .get_values()
                        .map(AdvancedBootdiskOptions::Lvm)
                })
                .flatten();

            if let Some(options) = options {
                siv.with_user_data(|opts: &mut InstallerOptions| {
                    opts.bootdisk.advanced = options;
                });

                add_next_screen(&timezone_dialog)(siv)
            } else {
                siv.add_layer(Dialog::info("Invalid values"));
            }
        }),
    )
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

fn timezone_dialog(siv: &mut Cursive) -> InstallerView {
    let options = siv
        .user_data::<InstallerOptions>()
        .map(|o| o.timezone.clone())
        .unwrap_or_default();

    let inner = LinearLayout::vertical()
        .child(FormInputView::new(
            "Country",
            EditView::new().content("Austria"),
        ))
        .child(FormInputView::new(
            "Timezone",
            EditView::new()
                .content(options.timezone)
                .with_name("timezone-tzname"),
        ))
        .child(FormInputView::new(
            "Keyboard layout",
            EditView::new()
                .content(options.kb_layout)
                .with_name("timezone-kblayout"),
        ));

    InstallerView::new(
        inner,
        Box::new(|siv| {
            let timezone = siv.call_on_name("timezone-tzname", |v: &mut EditView| {
                (*v.get_content()).clone()
            });

            let kb_layout = siv.call_on_name("timezone-kblayout", |v: &mut EditView| {
                (*v.get_content()).clone()
            });

            if let (Some(timezone), Some(kb_layout)) = (timezone, kb_layout) {
                siv.with_user_data(|opts: &mut InstallerOptions| {
                    opts.timezone = TimezoneOptions {
                        timezone,
                        kb_layout,
                    };
                });

                add_next_screen(&password_dialog)(siv);
            } else {
                siv.add_layer(Dialog::info("Invalid values"));
            }
        }),
    )
}

fn password_dialog(siv: &mut Cursive) -> InstallerView {
    let options = siv
        .user_data::<InstallerOptions>()
        .map(|o| o.password.clone())
        .unwrap_or_default();

    let inner = LinearLayout::vertical()
        .child(FormInputView::new(
            "Root password",
            EditView::new()
                .secret()
                .with_name("password-dialog-root-pw"),
        ))
        .child(FormInputView::new(
            "Confirm root password",
            EditView::new()
                .secret()
                .with_name("password-dialog-root-pw-confirm"),
        ))
        .child(FormInputView::new(
            "Administator email",
            EditView::new()
                .content(options.email)
                .with_name("password-dialog-email"),
        ));

    InstallerView::new(
        inner,
        Box::new(|siv| {
            // TODO: password validation
            add_next_screen(&network_dialog)(siv)
        }),
    )
}

fn network_dialog(siv: &mut Cursive) -> InstallerView {
    let options = siv
        .user_data::<InstallerOptions>()
        .map(|o| o.network.clone())
        .unwrap_or_default();

    let inner = LinearLayout::vertical()
        .child(FormInputView::new(
            "Management interface",
            SelectView::new().popup().with_all_str(vec!["eth0"]),
        ))
        .child(FormInputView::new(
            "Hostname (FQDN)",
            EditView::new().content(options.fqdn),
        ))
        .child(FormInputView::new(
            "IP address (CIDR)",
            LinearLayout::horizontal()
                .child(
                    EditView::new()
                        .content(options.ip_addr.to_string())
                        .full_width(),
                )
                .child(TextView::new(" / "))
                .child(
                    NumericEditView::new()
                        .max_value(32.)
                        .ints_only()
                        .max_content_width(2)
                        .fixed_width(3),
                ),
        ))
        .child(FormInputView::new(
            "Gateway address",
            EditView::new().content(options.gateway.to_string()),
        ))
        .child(FormInputView::new(
            "DNS serveraddress",
            EditView::new().content(options.dns_server.to_string()),
        ));

    InstallerView::new(inner, Box::new(|_| {}))
}
