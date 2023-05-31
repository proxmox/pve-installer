use std::str::FromStr;

use cursive::{
    event::{Event, EventResult},
    view::{Resizable, ViewWrapper},
    views::{DummyView, EditView, LinearLayout, ResizedView, TextView},
    View,
};

pub struct NumericEditView {
    view: EditView,
    max_value: Option<f64>,
}

impl NumericEditView {
    pub fn new() -> Self {
        Self {
            view: EditView::new().content("0."),
            max_value: None,
        }
    }

    pub fn max_value(mut self, max: f64) -> Self {
        self.max_value = Some(max);
        self
    }

    pub fn set_content(&mut self, content: f64) {
        self.view.set_content(content.to_string());
    }

    pub fn content(mut self, content: f64) -> Self {
        self.view = self.view.content(content.to_string());
        self
    }

    pub fn get_content(&self) -> Result<f64, <f64 as FromStr>::Err> {
        self.view.get_content().parse()
    }
}

impl ViewWrapper for NumericEditView {
    cursive::wrap_impl!(self.view: EditView);

    fn wrap_on_event(&mut self, event: Event) -> EventResult {
        let result = match event {
            Event::Char(c) if !(c.is_numeric() || c == '.') => EventResult::consumed(),
            _ => self.view.on_event(event),
        };

        if let Some(max) = self.max_value {
            if let Ok(val) = self.get_content() {
                if val > max {
                    let cb = self.view.remove(1);
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

pub struct DiskSizeFormInputView {
    view: LinearLayout,
}

impl DiskSizeFormInputView {
    pub fn new(label: &str) -> Self {
        let view = LinearLayout::horizontal()
            .child(TextView::new(format!("{label}: ")))
            .child(DummyView.full_width())
            .child(NumericEditView::new().full_width())
            .child(TextView::new(" GB"));

        Self { view }
    }

    pub fn content(mut self, content: u64) -> Self {
        let val = (content as f64) / 1024. / 1024.;

        let view = self
            .view
            .get_child_mut(2)
            .and_then(|v| v.downcast_mut::<ResizedView<NumericEditView>>());

        if let Some(view) = view {
            *view = NumericEditView::new().content(val).full_width();
        }

        self
    }

    pub fn get_content(&mut self) -> Option<u64> {
        self.with_view_mut(|v| {
            v.get_child_mut(2)?
                .downcast_mut::<ResizedView<NumericEditView>>()?
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
