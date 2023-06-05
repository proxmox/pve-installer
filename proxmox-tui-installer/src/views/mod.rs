mod table_view;

use cursive::{
    event::{Event, EventResult},
    view::{Resizable, ViewWrapper},
    views::{DummyView, EditView, LinearLayout, ResizedView, SelectView, TextView},
    View,
};
use std::{marker::PhantomData, rc::Rc, str::FromStr};

use crate::utils::CidrAddress;

pub use self::table_view::*;

pub struct NumericEditView<T> {
    view: EditView,
    max_value: Option<T>,
}

impl<T: Copy + ToString + FromStr + PartialOrd> NumericEditView<T> {
    pub fn new() -> Self {
        Self {
            view: EditView::new().content("0."),
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

pub struct DiskSizeFormInputView {
    view: LinearLayout,
}

impl DiskSizeFormInputView {
    pub fn new(label: &str) -> Self {
        let view = LinearLayout::horizontal()
            .child(TextView::new(format!("{label}: ")))
            .child(DummyView.full_width())
            .child(FloatEditView::new().full_width())
            .child(TextView::new(" GB"));

        Self { view }
    }

    pub fn content(mut self, content: u64) -> Self {
        let val = (content as f64) / 1024. / 1024.;

        if let Some(view) = self.view.get_child_mut(2).and_then(|v| v.downcast_mut()) {
            *view = FloatEditView::new().content(val).full_width();
        }

        self
    }

    pub fn get_content(&mut self) -> Option<u64> {
        self.with_view_mut(|v| {
            v.get_child_mut(2)?
                .downcast_mut::<ResizedView<FloatEditView>>()?
                .with_view_mut(|v| v.get_content().ok().map(|val| (val * 1024. * 1024.) as u64))?
        })
        .flatten()
    }
}

impl ViewWrapper for DiskSizeFormInputView {
    cursive::wrap_impl!(self.view: LinearLayout);
}

pub trait FormInputViewGetValue<R> {
    fn get_value(&self) -> Option<R>;
}

pub struct FormInputView<T: View> {
    view: LinearLayout,
    panthom: PhantomData<T>,
}

impl<T: View> FormInputView<T> {
    pub fn new(label: &str, input: T) -> Self {
        let view = LinearLayout::horizontal()
            .child(TextView::new(format!("{label}: ")))
            .child(DummyView.full_width())
            .child(input.full_width());

        Self {
            view,
            panthom: PhantomData,
        }
    }

    fn inner_input(&self) -> Option<&T> {
        self.view
            .get_child(2)?
            .downcast_ref::<ResizedView<T>>()
            .map(|v| v.get_inner())
    }
}

impl FormInputViewGetValue<String> for FormInputView<EditView> {
    fn get_value(&self) -> Option<String> {
        self.inner_input().map(|v| (*v.get_content()).clone())
    }
}

impl FormInputViewGetValue<String> for FormInputView<SelectView> {
    fn get_value(&self) -> Option<String> {
        self.inner_input()
            .and_then(|v| v.selection())
            .map(|v| (*v).clone())
    }
}

impl FormInputViewGetValue<CidrAddress> for FormInputView<CidrAddressEditView> {
    fn get_value(&self) -> Option<CidrAddress> {
        self.inner_input().and_then(|v| v.get_values())
    }
}

impl<T: View> ViewWrapper for FormInputView<T> {
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
            .parse()
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
