use cursive::{
    event::{Event, EventResult},
    Printer, Vec2, View,
};

pub trait TableViewItem {
    fn get_column(&self, name: &str) -> String;
}

struct TableViewColumn {
    name: String,
    title: String,
}

pub struct TableView<T: TableViewItem> {
    columns: Vec<TableViewColumn>,
    items: Vec<T>,
}

impl<T: TableViewItem> TableView<T> {
    pub fn new() -> Self {
        Self {
            columns: vec![],
            items: vec![],
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
            p.print(start + ((width - item.len()) / 2, 0), &item);

            start.x += width;
            p.print(start, "┆");
        }

        // Clear out the last delimiter again
        p.print(start, " ");
    }
}

impl<T: TableViewItem + 'static> View for TableView<T> {
    fn draw(&self, p: &Printer) {
        let col_width = (p.output_size.x - 1) / self.columns.len();

        Self::draw_row(p, self.columns.iter().map(|c| c.title.clone()), col_width);
        p.print_hdelim(Vec2::new(0, 1), p.output_size.x);

        let mut start = Vec2::new(0, 2);
        for row in &self.items {
            Self::draw_row(
                &p.offset(start),
                self.columns.iter().map(|c| row.get_column(&c.name)),
                col_width,
            );

            start.y += 1;
        }
    }

    fn on_event(&mut self, _: Event) -> EventResult {
        EventResult::Ignored
    }

    fn required_size(&mut self, constraint: Vec2) -> Vec2 {
        Vec2::new(constraint.x, self.items.len() + 2)
    }
}
