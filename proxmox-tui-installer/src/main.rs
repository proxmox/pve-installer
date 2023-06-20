#![forbid(unsafe_code)]

use std::{collections::HashMap, env, net::IpAddr, path::PathBuf};

use cursive::{
    event::Event,
    theme::{ColorStyle, Effect, PaletteColor, Style},
    view::{Nameable, Offset, Resizable, ViewWrapper},
    views::{
        Button, Checkbox, Dialog, DummyView, EditView, Layer, LinearLayout, PaddedView, Panel,
        ProgressBar, ResizedView, ScrollView, SelectView, StackView, TextContent, TextView,
        ViewRef,
    },
    Cursive, CursiveRunnable, ScreenId, View, XY,
};

mod options;
use options::*;

mod setup;
use setup::{LocaleInfo, RuntimeInfo, SetupInfo};

mod system;

mod utils;
use utils::Fqdn;

mod views;
use views::{
    BootdiskOptionsView, CidrAddressEditView, FormView, TableView, TableViewItem,
    TimezoneOptionsView,
};

// TextView::center() seems to garble the first two lines, so fix it manually here.
const PROXMOX_LOGO: &str = r#"
    ____
   / __ \_________  _  ______ ___  ____  _  __
  / /_/ / ___/ __ \| |/_/ __ `__ \/ __ \| |/_/
 / ____/ /  / /_/ />  </ / / / / / /_/ />  <
/_/   /_/   \____/_/|_/_/ /_/ /_/\____/_/|_|  "#;

struct InstallerView {
    view: ResizedView<Dialog>,
}

impl InstallerView {
    pub fn new<T: View>(
        state: &InstallerState,
        view: T,
        next_cb: Box<dyn Fn(&mut Cursive)>,
    ) -> Self {
        let inner = LinearLayout::vertical()
            .child(PaddedView::lrtb(0, 0, 1, 1, view))
            .child(PaddedView::lrtb(
                1,
                1,
                0,
                0,
                LinearLayout::horizontal()
                    .child(abort_install_button())
                    .child(DummyView.full_width())
                    .child(Button::new("Previous", switch_to_prev_screen))
                    .child(DummyView)
                    .child(Button::new("Next", next_cb)),
            ));

        Self::with_raw(state, inner)
    }

    pub fn with_raw(state: &InstallerState, view: impl View) -> Self {
        let setup = &state.setup_info;

        let title = format!(
            "{} ({}-{}) Installer",
            setup.config.fullname, setup.iso_info.release, setup.iso_info.isorelease
        );

        let inner = Dialog::around(view).title(title);

        Self {
            // Limit the maximum to something reasonable, such that it won't get spread out much
            // depending on the screen.
            view: ResizedView::with_max_size((120, 40), inner),
        }
    }
}

impl ViewWrapper for InstallerView {
    cursive::wrap_impl!(self.view: ResizedView<Dialog>);
}

struct InstallerBackgroundView {
    view: StackView,
}

impl InstallerBackgroundView {
    pub fn new(state: &InstallerState) -> Self {
        let product_name = state
            .setup_info
            .config
            .fullname
            .trim_start_matches("Proxmox ");

        let logo = format!("{PROXMOX_LOGO} {product_name}");
        let style = Style {
            effects: Effect::Bold.into(),
            color: ColorStyle::back(PaletteColor::View),
        };

        let mut view = StackView::new();
        view.add_fullscreen_layer(Layer::with_color(
            DummyView
                .full_width()
                .fixed_height(logo.lines().count() + 1),
            ColorStyle::back(PaletteColor::View),
        ));
        view.add_transparent_layer_at(
            XY {
                x: Offset::Center,
                y: Offset::Absolute(0),
            },
            TextView::new(logo).style(style),
        );

        Self { view }
    }
}

impl ViewWrapper for InstallerBackgroundView {
    cursive::wrap_impl!(self.view: StackView);
}

#[derive(Clone, Eq, Hash, PartialEq)]
enum InstallerStep {
    Licence,
    Bootdisk,
    Timezone,
    Password,
    Network,
    Summary,
    Install,
}

#[derive(Clone)]
struct InstallerState {
    options: InstallerOptions,
    setup_info: SetupInfo,
    runtime_info: RuntimeInfo,
    locales: LocaleInfo,
    steps: HashMap<InstallerStep, ScreenId>,
    in_test_mode: bool,
}

fn main() {
    let mut siv = cursive::termion();

    let in_test_mode = match env::args().nth(1).as_deref() {
        Some("-t") => true,

        // Always force the test directory in debug builds
        #[cfg(debug_assertions)]
        _ => true,

        #[cfg(not(debug_assertions))]
        _ => false,
    };

    let (setup_info, locales, runtime_info) = match installer_setup(in_test_mode) {
        Ok(result) => result,
        Err(err) => initial_setup_error(&mut siv, &err),
    };

    siv.clear_global_callbacks(Event::CtrlChar('c'));
    siv.set_on_pre_event(Event::CtrlChar('c'), trigger_abort_install_dialog);

    siv.set_user_data(InstallerState {
        options: InstallerOptions {
            bootdisk: BootdiskOptions::defaults_from(&runtime_info.disks[0]),
            timezone: TimezoneOptions::defaults_from(&runtime_info, &locales),
            password: PasswordOptions::default(),
            network: NetworkOptions::from(&runtime_info.network),
            reboot: false,
        },
        setup_info,
        runtime_info,
        locales,
        steps: HashMap::new(),
        in_test_mode,
    });

    switch_to_next_screen(&mut siv, InstallerStep::Licence, &license_dialog);
    siv.run();
}

fn installer_setup(in_test_mode: bool) -> Result<(SetupInfo, LocaleInfo, RuntimeInfo), String> {
    system::has_min_requirements()?;

    let base_path = if in_test_mode { "./testdir" } else { "/" };
    let mut path = PathBuf::from(base_path);

    path.push("run");
    path.push("proxmox-installer");

    let installer_info = {
        let mut path = path.clone();
        path.push("iso-info.json");

        setup::read_json(&path).map_err(|err| format!("Failed to retrieve setup info: {err}"))?
    };

    let locale_info = {
        let mut path = path.clone();
        path.push("locales.json");

        setup::read_json(&path).map_err(|err| format!("Failed to retrieve locale info: {err}"))?
    };

    let runtime_info: RuntimeInfo = {
        let mut path = path.clone();
        path.push("run-env-info.json");

        setup::read_json(&path)
            .map_err(|err| format!("Failed to retrieve runtime environment info: {err}"))?
    };

    if runtime_info.disks.is_empty() {
        Err("The installer could not find any supported hard disks.".to_owned())
    } else {
        Ok((installer_info, locale_info, runtime_info))
    }
}

fn initial_setup_error(siv: &mut CursiveRunnable, message: &str) -> ! {
    siv.add_layer(
        Dialog::around(TextView::new(message))
            .title("Installer setup error")
            .button("Ok", Cursive::quit),
    );
    siv.run();

    std::process::exit(1);
}

fn switch_to_next_screen(
    siv: &mut Cursive,
    step: InstallerStep,
    constructor: &dyn Fn(&mut Cursive) -> InstallerView,
) {
    let state = siv.user_data::<InstallerState>().cloned().unwrap();

    // Check if the screen already exists; if yes, then simply switch to it.
    if let Some(screen_id) = state.steps.get(&step) {
        siv.set_screen(*screen_id);
        return;
    }

    let v = constructor(siv);
    let screen = siv.add_active_screen();
    siv.with_user_data(|state: &mut InstallerState| state.steps.insert(step, screen));

    if let Some(state) = siv.user_data::<InstallerState>().cloned() {
        siv.screen_mut().add_transparent_layer_at(
            XY {
                x: Offset::Parent(0),
                y: Offset::Parent(0),
            },
            InstallerBackgroundView::new(&state),
        );
    }

    siv.screen_mut().add_layer(v);
}

fn switch_to_prev_screen(siv: &mut Cursive) {
    let id = siv.active_screen().saturating_sub(1);
    siv.set_screen(id);
}

#[cfg(not(debug_assertions))]
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

fn license_dialog(siv: &mut Cursive) -> InstallerView {
    let state = siv.user_data::<InstallerState>().unwrap();

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
                .child(Button::new("I agree", |siv| {
                    switch_to_next_screen(siv, InstallerStep::Bootdisk, &bootdisk_dialog)
                })),
        ));

    InstallerView::with_raw(state, inner)
}

fn bootdisk_dialog(siv: &mut Cursive) -> InstallerView {
    let state = siv.user_data::<InstallerState>().cloned().unwrap();

    InstallerView::new(
        &state,
        BootdiskOptionsView::new(&state.runtime_info.disks, &state.options.bootdisk)
            .with_name("bootdisk-options"),
        Box::new(|siv| {
            let options = siv
                .call_on_name("bootdisk-options", BootdiskOptionsView::get_values)
                .flatten();

            if let Some(options) = options {
                siv.with_user_data(|state: &mut InstallerState| {
                    state.options.bootdisk = options;
                });

                switch_to_next_screen(siv, InstallerStep::Timezone, &timezone_dialog);
            } else {
                siv.add_layer(Dialog::info("Invalid values"));
            }
        }),
    )
}

fn timezone_dialog(siv: &mut Cursive) -> InstallerView {
    let state = siv.user_data::<InstallerState>().unwrap();
    let options = &state.options.timezone;

    InstallerView::new(
        state,
        TimezoneOptionsView::new(&state.locales, options).with_name("timezone-options"),
        Box::new(|siv| {
            let options = siv.call_on_name("timezone-options", TimezoneOptionsView::get_values);

            match options {
                Some(Ok(options)) => {
                    siv.with_user_data(|state: &mut InstallerState| {
                        state.options.timezone = options;
                    });

                    switch_to_next_screen(siv, InstallerStep::Password, &password_dialog);
                }
                Some(Err(err)) => siv.add_layer(Dialog::info(format!("Invalid values: {err}"))),
                _ => siv.add_layer(Dialog::info("Invalid values")),
            }
        }),
    )
}

fn password_dialog(siv: &mut Cursive) -> InstallerView {
    let state = siv.user_data::<InstallerState>().unwrap();
    let options = &state.options.password;

    let inner = FormView::new()
        .child("Root password", EditView::new().secret())
        .child("Confirm root password", EditView::new().secret())
        .child(
            "Administator email",
            EditView::new().content(&options.email),
        )
        .with_name("password-options");

    InstallerView::new(
        state,
        inner,
        Box::new(|siv| {
            let options = siv.call_on_name("password-options", |view: &mut FormView| {
                let root_password = view
                    .get_value::<EditView, _>(0)
                    .ok_or("failed to retrieve password")?;

                let confirm_password = view
                    .get_value::<EditView, _>(1)
                    .ok_or("failed to retrieve password confirmation")?;

                let email = view
                    .get_value::<EditView, _>(2)
                    .ok_or("failed to retrieve email")?;

                if root_password.len() < 5 {
                    Err("password too short")
                } else if root_password != confirm_password {
                    Err("passwords do not match")
                } else if email == "mail@example.invalid" {
                    Err("invalid email address")
                } else {
                    Ok(PasswordOptions {
                        root_password,
                        email,
                    })
                }
            });

            match options {
                Some(Ok(options)) => {
                    siv.with_user_data(|state: &mut InstallerState| {
                        state.options.password = options;
                    });

                    switch_to_next_screen(siv, InstallerStep::Network, &network_dialog);
                }
                Some(Err(err)) => siv.add_layer(Dialog::info(format!("Invalid values: {err}"))),
                _ => siv.add_layer(Dialog::info("Invalid values")),
            }
        }),
    )
}

fn network_dialog(siv: &mut Cursive) -> InstallerView {
    let state = siv.user_data::<InstallerState>().unwrap();
    let options = &state.options.network;

    let inner = FormView::new()
        .child(
            "Management interface",
            SelectView::new()
                .popup()
                .with_all_str(state.runtime_info.network.interfaces.keys()),
        )
        .child(
            "Hostname (FQDN)",
            EditView::new().content(options.fqdn.to_string()),
        )
        .child(
            "IP address (CIDR)",
            CidrAddressEditView::new().content(options.address.clone()),
        )
        .child(
            "Gateway address",
            EditView::new().content(options.gateway.to_string()),
        )
        .child(
            "DNS server address",
            EditView::new().content(options.dns_server.to_string()),
        )
        .with_name("network-options");

    InstallerView::new(
        state,
        inner,
        Box::new(|siv| {
            let options = siv.call_on_name("network-options", |view: &mut FormView| {
                let ifname = view
                    .get_value::<SelectView, _>(0)
                    .ok_or("failed to retrieve management interface name")?;

                let fqdn = view
                    .get_value::<EditView, _>(1)
                    .ok_or("failed to retrieve host FQDN")?
                    .parse::<Fqdn>()
                    .map_err(|_| "failed to parse hostname".to_owned())?;

                let address = view
                    .get_value::<CidrAddressEditView, _>(2)
                    .ok_or("failed to retrieve host address")?;

                let gateway = view
                    .get_value::<EditView, _>(3)
                    .ok_or("failed to retrieve gateway address")?
                    .parse::<IpAddr>()
                    .map_err(|err| err.to_string())?;

                let dns_server = view
                    .get_value::<EditView, _>(3)
                    .ok_or("failed to retrieve DNS server address")?
                    .parse::<IpAddr>()
                    .map_err(|err| err.to_string())?;

                if address.addr().is_ipv4() != gateway.is_ipv4() {
                    Err("host and gateway IP address version must not differ".to_owned())
                } else if address.addr().is_ipv4() != dns_server.is_ipv4() {
                    Err("host and DNS IP address version must not differ".to_owned())
                } else if fqdn.to_string().chars().all(|c| c.is_ascii_digit()) {
                    // Not supported/allowed on Debian
                    Err("hostname cannot be purely numeric".to_owned())
                } else if fqdn.to_string().ends_with(".invalid") {
                    Err("hostname does not look valid".to_owned())
                } else {
                    Ok(NetworkOptions {
                        ifname,
                        fqdn,
                        address,
                        gateway,
                        dns_server,
                    })
                }
            });

            match options {
                Some(Ok(options)) => {
                    siv.with_user_data(|state: &mut InstallerState| {
                        state.options.network = options;
                    });

                    switch_to_next_screen(siv, InstallerStep::Summary, &summary_dialog);
                }
                Some(Err(err)) => siv.add_layer(Dialog::info(format!("Invalid values: {err}"))),
                _ => siv.add_layer(Dialog::info("Invalid values")),
            }
        }),
    )
}

pub struct SummaryOption {
    name: &'static str,
    value: String,
}

impl SummaryOption {
    pub fn new<S: Into<String>>(name: &'static str, value: S) -> Self {
        Self {
            name,
            value: value.into(),
        }
    }
}

impl TableViewItem for SummaryOption {
    fn get_column(&self, name: &str) -> String {
        match name {
            "name" => self.name.to_owned(),
            "value" => self.value.clone(),
            _ => unreachable!(),
        }
    }
}

fn summary_dialog(siv: &mut Cursive) -> InstallerView {
    let state = siv.user_data::<InstallerState>().unwrap();

    let inner = LinearLayout::vertical()
        .child(PaddedView::lrtb(
            0,
            0,
            1,
            2,
            TableView::new()
                .columns(&[
                    ("name".to_owned(), "Option".to_owned()),
                    ("value".to_owned(), "Selected value".to_owned()),
                ])
                .items(state.options.to_summary(&state.locales)),
        ))
        .child(
            LinearLayout::horizontal()
                .child(DummyView.full_width())
                .child(Checkbox::new().with_name("reboot-after-install"))
                .child(
                    TextView::new(" Automatically reboot after successful installation").no_wrap(),
                )
                .child(DummyView.full_width()),
        )
        .child(PaddedView::lrtb(
            1,
            1,
            1,
            0,
            LinearLayout::horizontal()
                .child(abort_install_button())
                .child(DummyView.full_width())
                .child(Button::new("Previous", switch_to_prev_screen))
                .child(DummyView)
                .child(Button::new("Install", |siv| {
                    let reboot = siv
                        .find_name("reboot-after-install")
                        .map(|v: ViewRef<Checkbox>| v.is_checked())
                        .unwrap_or_default();

                    siv.with_user_data(|state: &mut InstallerState| {
                        state.options.reboot = reboot;
                    });

                    switch_to_next_screen(siv, InstallerStep::Install, &install_progress_dialog);
                })),
        ));

    InstallerView::with_raw(state, inner)
}

fn install_progress_dialog(siv: &mut Cursive) -> InstallerView {
    // Ensure the screen is updated independently of keyboard events and such
    siv.set_autorefresh(true);

    let state = siv.user_data::<InstallerState>().unwrap();
    let progress_text = TextContent::new("extracting ..");
    let progress_bar = ProgressBar::new()
        .with_task({
            move |counter| {
                for _ in 0..100 {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    counter.tick(1);
                }
            }
        })
        .full_width();

    let inner = PaddedView::lrtb(
        1,
        1,
        1,
        1,
        LinearLayout::vertical()
            .child(PaddedView::lrtb(1, 1, 0, 0, progress_bar))
            .child(DummyView)
            .child(TextView::new_with_content(progress_text).center()),
    );

    InstallerView::with_raw(state, inner)
}
