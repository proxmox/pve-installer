use std::borrow::{Borrow, BorrowMut};

use cursive::{
    Printer, Vec2, View,
    direction::Direction,
    event::{AnyCb, Event, EventResult, Key},
    theme::{ColorStyle, PaletteColor},
    utils::{markup::StyledString, span::SpannedStr},
    view::{CannotFocus, IntoBoxedView, Selector, ViewNotFound},
};

pub struct TabbedView {
    /// All tab views in format (name, view)
    views: Vec<(String, Box<dyn View>)>,
    /// Currently active tab index
    current: usize,
    /// Whether the tab bar has focus currently
    bar_has_focus: bool,
}

impl TabbedView {
    /// Creates a view with multiple tabs.
    pub fn new() -> Self {
        Self {
            views: vec![],
            current: 0,
            bar_has_focus: false,
        }
    }

    /// Adds a tab to the view. The `name` is the string displayed at the top.
    ///
    /// Chainable variant.
    pub fn tab<V>(mut self, name: &str, view: V) -> Self
    where
        V: 'static + IntoBoxedView,
    {
        assert!(!name.is_empty());
        self.views.push((name.to_owned(), view.into_boxed_view()));
        self
    }

    /// Returns a reference to the specified tab content view.
    pub fn get(&self, index: usize) -> Option<&dyn View> {
        self.views.get(index).map(|(_, view)| view.borrow())
    }

    /// Returns a mutable reference to the specified tab content view.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut dyn View> {
        self.views.get_mut(index).map(|(_, view)| view.borrow_mut())
    }

    /// Draws the border around the tab view and the name header for each tab.
    fn draw_border(&self, p: &Printer) {
        let names_len: usize = self.views.iter().map(|(name, _)| name.len() + 2).sum();
        let tabbar_width = names_len + self.views.len() + 1;

        let top_border_width = (p.output_size.x - tabbar_width) / 2;
        p.print_box((0, 0), p.output_size, false);

        self.print_tab_names(p.offset((top_border_width, 0)));
    }

    /// Draws all tab names with appropriate highlighting, depending on its state,
    /// to the specified printer `p` at `(0, 0)`.
    fn print_tab_names(&self, p: Printer) {
        let mut pos = Vec2::zero();
        for (index, name) in self.views.iter().map(|(name, _)| name).enumerate() {
            p.print(pos, "| ");
            pos.x += 2;

            if index == self.current {
                self.print_active_tab_name(name, p.offset(pos));
            } else {
                p.print(pos, name);
            }

            pos.x += name.len();
            p.print(pos, " ");
            pos.x += 1;

            p.print(pos, "|");
        }
    }

    /// Draws the active tab name to the printer `p`, with its highlighting
    /// additionally depending upon whether the tab bar currently has focus or not.
    fn print_active_tab_name(&self, name: &str, p: Printer) {
        let background = if self.bar_has_focus {
            PaletteColor::Highlight
        } else {
            PaletteColor::HighlightInactive
        };

        p.print_styled(
            (0, 0),
            SpannedStr::from(&StyledString::styled(
                name,
                ColorStyle::new(PaletteColor::HighlightText, background),
            )),
        )
    }
}

impl View for TabbedView {
    fn draw(&self, printer: &Printer) {
        self.draw_border(printer);

        if let Some(view) = self.get(self.current) {
            view.draw(&printer.offset((1, 1)).focused(!self.bar_has_focus));
        }
    }

    fn layout(&mut self, size: Vec2) {
        for (_, view) in self.views.iter_mut() {
            view.layout(size.saturating_sub((2, 2)));
        }
    }

    fn required_size(&mut self, constraint: Vec2) -> Vec2 {
        if let Some(view) = self.get_mut(self.current) {
            view.required_size(constraint) + (2, 2)
        } else {
            constraint
        }
    }

    fn on_event(&mut self, event: Event) -> EventResult {
        match event {
            Event::Key(Key::Right) if self.bar_has_focus => {
                self.current = (self.current + 1) % self.views.len();
                EventResult::consumed()
            }
            Event::Key(Key::Left) if self.bar_has_focus => {
                self.current = if self.current == 0 {
                    self.views.len() - 1
                } else {
                    self.current - 1
                };
                EventResult::consumed()
            }
            Event::Key(Key::Down) if self.bar_has_focus => {
                self.bar_has_focus = false;
                self.get_mut(self.current)
                    .and_then(|v| v.take_focus(Direction::up()).ok())
                    .unwrap_or(EventResult::Ignored)
            }
            Event::Key(Key::Up) if self.bar_has_focus => EventResult::Ignored,
            Event::FocusLost if self.bar_has_focus => {
                self.bar_has_focus = false;
                EventResult::consumed()
            }
            Event::Key(Key::Up) if !self.bar_has_focus => {
                let result = self
                    .get_mut(self.current)
                    .map(|v| v.on_event(event))
                    .unwrap_or(EventResult::Ignored);

                match result {
                    EventResult::Ignored => {
                        self.bar_has_focus = true;
                        if let Some(view) = self.get_mut(self.current) {
                            view.on_event(Event::FocusLost);
                        }
                        EventResult::consumed()
                    }
                    ev => ev,
                }
            }
            _ if !self.bar_has_focus => self
                .get_mut(self.current)
                .map(|v| v.on_event(event))
                .unwrap_or_else(EventResult::consumed),
            _ => EventResult::Ignored,
        }
    }

    fn call_on_any(&mut self, selector: &Selector, callback: AnyCb) {
        for (_, view) in &mut self.views {
            view.call_on_any(selector, callback);
        }
    }

    fn focus_view(&mut self, selector: &Selector) -> Result<EventResult, ViewNotFound> {
        if let Some(view) = self.get_mut(self.current) {
            view.focus_view(selector)
        } else {
            Err(ViewNotFound)
        }
    }

    fn take_focus(&mut self, direction: Direction) -> Result<EventResult, CannotFocus> {
        self.bar_has_focus = direction == Direction::up();
        Ok(EventResult::consumed())
    }
}
