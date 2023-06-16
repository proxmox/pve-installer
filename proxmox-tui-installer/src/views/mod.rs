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
}

impl<T: Copy + ToString + FromStr + PartialOrd> NumericEditView<T> {
    pub fn new() -> Self {
        Self {
            view: EditView::new().content("0"),
            max_value: None,
        }
    }

    pub fn max_value(mut self, max: T) -> Self {
        self.max_value = Some(max);
        self
    }

    pub fn max_content_width(mut self, width: usize) -> Self {
        self.view = self.view.max_content_width(width);
        self
    }

    pub fn content(mut self, content: T) -> Self {
        self.view = self.view.content(content.to_string());
        self
    }

    pub fn get_content(&self) -> Result<T, <T as FromStr>::Err> {
        self.view.get_content().parse()
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

        let result = match event {
            // Drop all other characters than numbers; allow dots if not set to integer-only
            Event::Char(c) if !(c.is_numeric() || c == '.') => EventResult::consumed(),
            _ => self.view.on_event(event),
        };

        self.check_bounds(original, result)
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

pub struct DiskSizeEditView {
    view: LinearLayout,
}

impl DiskSizeEditView {
    pub fn new() -> Self {
        let view = LinearLayout::horizontal()
            .child(FloatEditView::new().full_width())
            .child(TextView::new(" GB"));

        Self { view }
    }

    pub fn content(mut self, content: u64) -> Self {
        let val = (content as f64) / 1024. / 1024. / 1024.;

        if let Some(view) = self.view.get_child_mut(0).and_then(|v| v.downcast_mut()) {
            *view = FloatEditView::new().content(val).full_width();
        }

        self
    }

    pub fn get_content(&self) -> Option<u64> {
        self.with_view(|v| {
            v.get_child(0)?
                .downcast_ref::<ResizedView<FloatEditView>>()?
                .with_view(|v| {
                    v.get_content()
                        .ok()
                        .map(|val| (val * 1024. * 1024. * 1024.) as u64)
                })?
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

impl FormViewGetValue<u64> for DiskSizeEditView {
    fn get_value(&self) -> Option<u64> {
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
            .max_value(32)
            .max_content_width(2)
            .content(content)
            .fixed_width(3)
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
