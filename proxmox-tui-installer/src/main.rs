#![forbid(unsafe_code)]

use cursive::{
    view::{Resizable, ViewWrapper},
    views::{
        Button, Dialog, DummyView, LinearLayout, PaddedView, ResizedView, ScrollView, TextView,
    },
    Cursive, View,
};

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
    view: LinearLayout,
}

impl InstallerView {
    pub fn new<T: View>(view: T) -> Self {
        Self {
            view: LinearLayout::vertical()
                .child(PaddedView::lrtb(1, 1, 0, 1, TextView::new(LOGO).center()))
                .child(Dialog::around(view).title(TITLE)),
        }
    }
}

impl ViewWrapper for InstallerView {
    cursive::wrap_impl!(self.view: LinearLayout);
}

fn main() {
    let mut siv = cursive::termion();

    siv.add_active_screen();
    siv.screen_mut().add_layer(license_dialog());
    siv.run();
}

fn add_next_screen(constructor: &dyn Fn() -> InstallerView) -> Box<dyn Fn(&mut Cursive) + '_> {
    Box::new(|siv: &mut Cursive| {
        siv.add_active_screen();
        siv.screen_mut().add_layer(constructor());
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

fn abort_install_button() -> Button {
    Button::new("Abort", |siv| {
        yes_no_dialog(
            siv,
            "Abort installation?",
            "Are you sure you want to abort the installation?",
            &Cursive::quit,
        )
    })
}

fn get_eula() -> String {
    #[cfg(debug_assertions)]
    "< Debug build - ignoring non-existing EULA >".to_owned()
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
        .child(Dialog::around(ResizedView::with_max_size(
            (120, 25),
            ScrollView::new(TextView::new(get_eula())),
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

fn bootdisk_dialog() -> InstallerView {
    InstallerView::new(DummyView)
}
