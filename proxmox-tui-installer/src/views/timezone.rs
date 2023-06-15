use super::FormView;
use crate::{options::TimezoneOptions, setup::LocaleInfo};
use cursive::{
    view::{Nameable, ViewWrapper},
    views::{EditView, NamedView, SelectView},
    Cursive,
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

        let timezones = locales.cczones.get(&countries[0].1);

        let country_selectview = SelectView::new()
            .popup()
            .with_all(countries.clone())
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

        let view = FormView::new()
            .child("Country", country_selectview)
            .child(
                "Timezone",
                Self::timezone_selectview(timezones.unwrap_or(&vec![]))
                    .with_name("timezone-options-tz"),
            )
            .child(
                "Keyboard layout",
                EditView::new().content(&options.kb_layout),
            );

        Self { view }
    }

    pub fn get_values(&mut self) -> Result<TimezoneOptions, String> {
        let timezone = self
            .view
            .get_value::<NamedView<SelectView>, _>(1)
            .ok_or("failed to retrieve timezone")?;

        let kb_layout = self
            .view
            .get_value::<EditView, _>(2)
            .ok_or("failed to retrieve keyboard layout")?;

        Ok(TimezoneOptions {
            timezone,
            kb_layout,
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
