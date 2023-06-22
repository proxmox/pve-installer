use std::{net::IpAddr, rc::Rc, str::FromStr};

use cursive::{
    event::{Event, EventResult},
    view::{Resizable, ViewWrapper},
    views::{EditView, LinearLayout, NamedView, ResizedView, SelectView, TextView},
    View,
};

use crate::utils::CidrAddress;

mod bootdisk;
pub use bootdisk::*;

mod table_view;
pub use table_view::*;

mod timezone;
pub use timezone::*;

pub struct NumericEditView<T> {
    view: EditView,
    max_value: Option<T>,
    max_content_width: Option<usize>,
    allow_empty: bool,
}

impl<T: Copy + ToString + FromStr + PartialOrd> NumericEditView<T> {
    pub fn new() -> Self {
        Self {
            view: EditView::new().content("0"),
            max_value: None,
            max_content_width: None,
            allow_empty: false,
        }
    }

    pub fn max_value(mut self, max: T) -> Self {
        self.max_value = Some(max);
        self
    }

    pub fn max_content_width(mut self, width: usize) -> Self {
        self.max_content_width = Some(width);
        self.view.set_max_content_width(self.max_content_width);
        self
    }

    pub fn allow_empty(mut self, value: bool) -> Self {
        self.allow_empty = value;

        if value {
            self.view = EditView::new();
        } else {
            self.view = EditView::new().content("0");
        }

        self.view.set_max_content_width(self.max_content_width);
        self
    }

    pub fn get_content(&self) -> Result<T, <T as FromStr>::Err> {
        assert!(!self.allow_empty);
        self.view.get_content().parse()
    }

    pub fn get_content_maybe(&self) -> Option<Result<T, <T as FromStr>::Err>> {
        let content = self.view.get_content();
        if !content.is_empty() {
            Some(self.view.get_content().parse())
        } else {
            None
        }
    }

    pub fn set_max_value(&mut self, max: T) {
        self.max_value = Some(max);
    }

    fn check_bounds(&mut self, original: Rc<String>, result: EventResult) -> EventResult {
        // Check if the new value is actually valid according to the max value, if set
        if let Some(max) = self.max_value {
            if let Ok(val) = self.get_content() {
                if result.is_consumed() && val > max {
                    // Restore the original value, before the insert
                    let cb = self.view.set_content((*original).clone());
                    return EventResult::with_cb_once(move |siv| {
                        result.process(siv);
                        cb(siv);
                    });
                }
            }
        }

        result
    }
}

pub type FloatEditView = NumericEditView<f64>;
pub type IntegerEditView = NumericEditView<usize>;

impl ViewWrapper for FloatEditView {
    cursive::wrap_impl!(self.view: EditView);

    fn wrap_on_event(&mut self, event: Event) -> EventResult {
        let original = self.view.get_content();

        let has_decimal_place = original.find('.').is_some();

        let result = match event {
            Event::Char(c) if !c.is_numeric() && c != '.' => return EventResult::consumed(),
            Event::Char('.') if has_decimal_place => return EventResult::consumed(),
            _ => self.view.on_event(event),
        };

        let decimal_places = self
            .view
            .get_content()
            .split_once('.')
            .map(|(_, s)| s.len())
            .unwrap_or_default();
        if decimal_places > 2 {
            let cb = self.view.set_content((*original).clone());
            return EventResult::with_cb_once(move |siv| {
                result.process(siv);
                cb(siv);
            });
        }

        self.check_bounds(original, result)
    }
}

impl FloatEditView {
    pub fn content(mut self, content: f64) -> Self {
        self.view = self.view.content(format!("{:.2}", content));
        self
    }
}

impl ViewWrapper for IntegerEditView {
    cursive::wrap_impl!(self.view: EditView);

    fn wrap_on_event(&mut self, event: Event) -> EventResult {
        let original = self.view.get_content();

        let result = match event {
            // Drop all other characters than numbers; allow dots if not set to integer-only
            Event::Char(c) if !c.is_numeric() => EventResult::consumed(),
            _ => self.view.on_event(event),
        };

        self.check_bounds(original, result)
    }
}

impl IntegerEditView {
    pub fn content(mut self, content: usize) -> Self {
        self.view = self.view.content(content.to_string());
        self
    }
}

pub struct DiskSizeEditView {
    view: LinearLayout,
    allow_empty: bool,
}

impl DiskSizeEditView {
    pub fn new() -> Self {
        let view = LinearLayout::horizontal()
            .child(FloatEditView::new().full_width())
            .child(TextView::new(" GB"));

        Self {
            view,
            allow_empty: false,
        }
    }

    pub fn new_emptyable() -> Self {
        let view = LinearLayout::horizontal()
            .child(FloatEditView::new().allow_empty(true).full_width())
            .child(TextView::new(" GB"));

        Self {
            view,
            allow_empty: true,
        }
    }

    pub fn content(mut self, content: f64) -> Self {
        if let Some(view) = self.view.get_child_mut(0).and_then(|v| v.downcast_mut()) {
            *view = FloatEditView::new().content(content).full_width();
        }

        self
    }

    pub fn content_maybe(self, content: Option<f64>) -> Self {
        if let Some(value) = content {
            self.content(value)
        } else {
            self
        }
    }

    pub fn max_value(mut self, max: f64) -> Self {
        if let Some(view) = self
            .view
            .get_child_mut(0)
            .and_then(|v| v.downcast_mut::<ResizedView<FloatEditView>>())
        {
            view.get_inner_mut().set_max_value(max);
        }

        self
    }

    pub fn get_content(&self) -> Option<f64> {
        self.with_view(|v| {
            v.get_child(0)?
                .downcast_ref::<ResizedView<FloatEditView>>()?
                .with_view(|v| {
                    if self.allow_empty {
                        v.get_content_maybe().and_then(Result::ok)
                    } else {
                        v.get_content().ok()
                    }
                })
                .flatten()
        })
        .flatten()
    }
}

impl ViewWrapper for DiskSizeEditView {
    cursive::wrap_impl!(self.view: LinearLayout);
}

pub trait FormViewGetValue<R> {
    fn get_value(&self) -> Option<R>;
}

impl FormViewGetValue<String> for EditView {
    fn get_value(&self) -> Option<String> {
        Some((*self.get_content()).clone())
    }
}

impl<T: 'static + Clone> FormViewGetValue<T> for SelectView<T> {
    fn get_value(&self) -> Option<T> {
        self.selection().map(|v| (*v).clone())
    }
}

impl<T> FormViewGetValue<T> for NumericEditView<T>
where
    T: Copy + ToString + FromStr + PartialOrd,
    NumericEditView<T>: ViewWrapper,
{
    fn get_value(&self) -> Option<T> {
        self.get_content().ok()
    }
}

impl FormViewGetValue<CidrAddress> for CidrAddressEditView {
    fn get_value(&self) -> Option<CidrAddress> {
        self.get_values()
    }
}

impl<T, R> FormViewGetValue<R> for NamedView<T>
where
    T: 'static + FormViewGetValue<R>,
    NamedView<T>: ViewWrapper,
    <NamedView<T> as ViewWrapper>::V: FormViewGetValue<R>,
{
    fn get_value(&self) -> Option<R> {
        self.with_view(|v| v.get_value()).flatten()
    }
}

impl FormViewGetValue<f64> for DiskSizeEditView {
    fn get_value(&self) -> Option<f64> {
        self.get_content()
    }
}

pub struct FormView {
    view: LinearLayout,
}

impl FormView {
    pub fn new() -> Self {
        let view = LinearLayout::horizontal()
            .child(LinearLayout::vertical().full_width())
            .child(LinearLayout::vertical().full_width());

        Self { view }
    }

    pub fn add_child(&mut self, label: &str, view: impl View) {
        self.add_to_column(0, TextView::new(format!("{label}: ")));
        self.add_to_column(1, view);
    }

    pub fn child(mut self, label: &str, view: impl View) -> Self {
        self.add_child(label, view);
        self
    }

    pub fn get_child<T: View>(&self, index: usize) -> Option<&T> {
        self.view
            .get_child(1)?
            .downcast_ref::<ResizedView<LinearLayout>>()?
            .get_inner()
            .get_child(index)?
            .downcast_ref::<T>()
    }

    pub fn get_value<T, R>(&self, index: usize) -> Option<R>
    where
        T: View + FormViewGetValue<R>,
    {
        self.get_child::<T>(index)?.get_value()
    }

    pub fn replace_child(&mut self, index: usize, view: impl View) {
        let parent = self
            .view
            .get_child_mut(1)
            .and_then(|v| v.downcast_mut())
            .map(ResizedView::<LinearLayout>::get_inner_mut);

        if let Some(parent) = parent {
            parent.remove_child(index);
            parent.insert_child(index, view);
        }
    }

    pub fn len(&self) -> usize {
        self.view
            .get_child(1)
            .and_then(|v| v.downcast_ref::<ResizedView<LinearLayout>>())
            .unwrap()
            .get_inner()
            .len()
    }

    fn add_to_column(&mut self, index: usize, view: impl View) {
        self.view
            .get_child_mut(index)
            .and_then(|v| v.downcast_mut::<ResizedView<LinearLayout>>())
            .unwrap()
            .get_inner_mut()
            .add_child(view);
    }
}

impl ViewWrapper for FormView {
    cursive::wrap_impl!(self.view: LinearLayout);
}

pub struct CidrAddressEditView {
    view: LinearLayout,
}

impl CidrAddressEditView {
    pub fn new() -> Self {
        let view = LinearLayout::horizontal()
            .child(EditView::new().full_width())
            .child(TextView::new(" / "))
            .child(Self::mask_edit_view(0));

        Self { view }
    }

    pub fn content(mut self, cidr: CidrAddress) -> Self {
        if let Some(view) = self
            .view
            .get_child_mut(0)
            .and_then(|v| v.downcast_mut::<ResizedView<EditView>>())
        {
            *view = EditView::new()
                .content(cidr.addr().to_string())
                .full_width();
        }

        if let Some(view) = self
            .view
            .get_child_mut(2)
            .and_then(|v| v.downcast_mut::<ResizedView<IntegerEditView>>())
        {
            *view = Self::mask_edit_view(cidr.mask());
        }

        self
    }

    fn mask_edit_view(content: usize) -> ResizedView<IntegerEditView> {
        IntegerEditView::new()
            .max_value(128)
            .max_content_width(3)
            .content(content)
            .fixed_width(4)
    }

    fn get_values(&self) -> Option<CidrAddress> {
        let addr = self
            .view
            .get_child(0)?
            .downcast_ref::<ResizedView<EditView>>()?
            .get_inner()
            .get_content()
            .parse::<IpAddr>()
            .ok()?;

        let mask = self
            .view
            .get_child(2)?
            .downcast_ref::<ResizedView<IntegerEditView>>()?
            .get_inner()
            .get_content()
            .ok()?;

        CidrAddress::new(addr, mask).ok()
    }
}

impl ViewWrapper for CidrAddressEditView {
    cursive::wrap_impl!(self.view: LinearLayout);
}
