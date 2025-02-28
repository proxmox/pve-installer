use cursive::{
    Printer, Rect, Vec2, View,
    direction::Direction,
    event::{Event, EventResult},
    view::{CannotFocus, scroll},
};

const HEADER_HEIGHT: usize = 2;

pub trait TableViewItem {
    fn get_column(&self, name: &str) -> String;
}

struct TableViewColumn {
    name: String,
    title: String,
}

pub struct TableView<T> {
    columns: Vec<TableViewColumn>,
    items: Vec<T>,
    scroller: scroll::Core,
}

impl<T: TableViewItem> TableView<T> {
    pub fn new() -> Self {
        Self {
            columns: vec![],
            items: vec![],
            scroller: scroll::Core::new(),
        }
    }

    pub fn columns(mut self, columns: &[(String, String)]) -> Self {
        self.columns = columns
            .iter()
            .map(|(n, t)| TableViewColumn {
                name: n.clone(),
                title: t.clone(),
            })
            .collect();
        self
    }

    pub fn items(mut self, items: Vec<T>) -> Self {
        self.items = items;
        self
    }

    fn draw_row(p: &Printer, items: impl Iterator<Item = String>, width: usize) {
        let mut start = Vec2::zero();

        for item in items {
            p.print(start + (2, 0), &item);

            start.x += width;
            p.print(start, "┆");
        }

        // Clear out the last delimiter again
        p.print(start, " ");
    }

    fn draw_content_row(&self, p: &Printer, row: usize) {
        let contents = self
            .columns
            .iter()
            .map(|c| self.items[row].get_column(&c.name));
        let width = (p.size.x - 1) / self.columns.len();

        Self::draw_row(p, contents, width);
    }

    fn inner_required_size(&mut self, mut constraint: Vec2) -> Vec2 {
        // Clamp the inner height to at least 3 rows (header + separator + one row) and at max. to
        // (number of rows + header + separator)
        constraint.y = constraint
            .y
            .clamp(HEADER_HEIGHT + 1, self.items.len() + HEADER_HEIGHT);

        constraint
    }

    fn inner_important_area(&self, size: Vec2) -> Rect {
        // Mark header + separator + first row as important
        Rect::from_size((0, 0), (size.x, HEADER_HEIGHT + 1))
    }
}

impl<T: TableViewItem + 'static + Send + Sync> View for TableView<T> {
    fn draw(&self, p: &Printer) {
        // Equally split up the columns width, taking into account the scrollbar size and column
        // separator.
        let width = (p.size.x - self.scroller.scrollbar_size().x - 1) / self.columns.len();

        Self::draw_row(p, self.columns.iter().map(|c| c.title.clone()), width);
        p.print_hdelim((0, 1), p.size.x);

        scroll::draw_lines(self, &p.offset((0, HEADER_HEIGHT)), Self::draw_content_row);
    }

    // TODO: Pre-compute column sizes and cache contents in the layout phase, thus avoiding any
    // expensive operations in the draw phase.
    fn layout(&mut self, size: Vec2) {
        scroll::layout(
            self,
            size.saturating_sub((0, HEADER_HEIGHT)),
            false,
            |_, _| {},
            |s, req_size| Vec2::new(req_size.x, s.items.len()),
        )
    }

    fn needs_relayout(&self) -> bool {
        self.scroller.needs_relayout()
    }

    fn required_size(&mut self, constraint: Vec2) -> Vec2 {
        scroll::required_size(self, constraint, false, Self::inner_required_size)
    }

    fn on_event(&mut self, event: Event) -> EventResult {
        scroll::on_event(
            self,
            event.relativized((0, HEADER_HEIGHT)),
            |_, _| EventResult::Ignored,
            Self::inner_important_area,
        )
    }

    fn take_focus(&mut self, _: Direction) -> Result<EventResult, CannotFocus> {
        // Only take the focus if scrollbars are visible
        if self.scroller.is_scrolling().any() {
            Ok(EventResult::consumed())
        } else {
            Err(CannotFocus)
        }
    }

    fn important_area(&self, size: Vec2) -> Rect {
        scroll::important_area(self, size, Self::inner_important_area)
    }
}

cursive::impl_scroller!(TableView<T>::scroller);
