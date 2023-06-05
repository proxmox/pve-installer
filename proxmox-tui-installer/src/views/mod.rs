mod table_view;

use cursive::{
    event::{Event, EventResult},
    view::{Resizable, ViewWrapper},
    views::{DummyView, EditView, LinearLayout, ResizedView, TextView},
    View,
};
use std::{net::IpAddr, rc::Rc, str::FromStr};

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

pub struct FormInputView {
    view: LinearLayout,
}

impl FormInputView {
    pub fn new<T: View>(label: &str, input: T) -> Self {
        let view = LinearLayout::horizontal()
            .child(TextView::new(format!("{label}: ")))
            .child(DummyView.full_width())
            .child(input.full_width());

        Self { view }
    }
}

impl ViewWrapper for FormInputView {
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

    pub fn content(mut self, addr: IpAddr, mask: usize) -> Self {
        if let Some(view) = self
            .view
            .get_child_mut(0)
            .and_then(|v| v.downcast_mut::<ResizedView<EditView>>())
        {
            *view = EditView::new().content(addr.to_string()).full_width();
        }

        if let Some(view) = self
            .view
            .get_child_mut(2)
            .and_then(|v| v.downcast_mut::<ResizedView<IntegerEditView>>())
        {
            *view = Self::mask_edit_view(mask);
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
}

impl ViewWrapper for CidrAddressEditView {
    cursive::wrap_impl!(self.view: LinearLayout);
}
