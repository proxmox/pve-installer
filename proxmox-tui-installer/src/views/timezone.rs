use cursive::{
    view::{Nameable, ViewWrapper},
    views::{Dialog, NamedView, SelectView},
    Cursive,
};

use super::FormView;
use crate::{
    system, InstallerState,
};
use proxmox_installer_common::{
    options::TimezoneOptions,
    setup::{KeyboardMapping, LocaleInfo},
};

pub struct TimezoneOptionsView {
    view: FormView,
}

impl TimezoneOptionsView {
    pub fn new(locales: &LocaleInfo, options: &TimezoneOptions) -> Self {
        let mut countries = locales
            .countries
            .clone()
            .into_iter()
            .map(|(cc, c)| (c.name, cc))
            .collect::<Vec<(String, String)>>();
        countries.sort();

        let country_selection_pos = countries
            .iter()
            .position(|c| c.1 == options.country)
            .unwrap_or_default();

        let timezones = locales.cczones.get(&options.country);

        let country_selectview = SelectView::new()
            .popup()
            .with_all(countries.clone())
            .selected(country_selection_pos)
            .on_submit({
                let cczones = locales.cczones.clone();
                move |siv: &mut Cursive, selected: &String| {
                    siv.call_on_name("timezone-options-tz", {
                        let cczones = cczones.clone();
                        move |view: &mut SelectView| {
                            *view =
                                Self::timezone_selectview(cczones.get(selected).unwrap_or(&vec![]));
                        }
                    });
                }
            });

        let mut kb_layouts = locales
            .kmap
            .clone()
            .into_values()
            .map(|l| (l.name.clone(), l))
            .collect::<Vec<(String, KeyboardMapping)>>();
        kb_layouts.sort();

        let kb_layout_selected_pos = kb_layouts
            .iter()
            .position(|l| l.1.id == options.kb_layout)
            .unwrap_or_default();

        let view = FormView::new()
            .child("Country", country_selectview)
            .child(
                "Timezone",
                Self::timezone_selectview(timezones.unwrap_or(&vec![]))
                    .with_name("timezone-options-tz"),
            )
            .child(
                "Keyboard layout",
                SelectView::new()
                    .popup()
                    .with_all(kb_layouts)
                    .selected(kb_layout_selected_pos)
                    .on_submit(|siv, selected| {
                        if siv
                            .user_data::<InstallerState>()
                            .map(|state| state.in_test_mode)
                            // In doubt, don't override the layout
                            .unwrap_or(true)
                        {
                            return;
                        }

                        if let Err(err) = system::set_keyboard_layout(selected) {
                            siv.add_layer(Dialog::info(format!(
                                "Failed to apply keyboard layout: {err}"
                            )));
                        }
                    }),
            );

        Self { view }
    }

    pub fn get_values(&mut self) -> Result<TimezoneOptions, String> {
        let country = self
            .view
            .get_value::<SelectView, _>(0)
            .ok_or("failed to retrieve timezone")?;

        let timezone = self
            .view
            .get_value::<NamedView<SelectView>, _>(1)
            .ok_or("failed to retrieve timezone")?;

        let kmap = self
            .view
            .get_value::<SelectView<KeyboardMapping>, _>(2)
            .ok_or("failed to retrieve keyboard layout")?;

        Ok(TimezoneOptions {
            country,
            timezone,
            kb_layout: kmap.id,
        })
    }

    fn timezone_selectview(zones: &[String]) -> SelectView {
        let mut zones = zones.to_owned();
        zones.sort();
        // Ensure UTC is always last
        zones.push("UTC".to_string());

        SelectView::new().popup().with_all_str(zones)
    }
}

impl ViewWrapper for TimezoneOptionsView {
    cursive::wrap_impl!(self.view: FormView);
}
