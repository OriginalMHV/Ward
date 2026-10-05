use console::style;
use tabled::builder::Builder;
use tabled::settings::object::{Columns, Rows};
use tabled::settings::{Alignment, Modify, Style};

/// The green `[ok]` or red `[!!]` status marker used in status tables.
pub(crate) fn ok_icon(ok: bool) -> String {
    if ok {
        style("[ok]").green().to_string()
    } else {
        style("[!!]").red().to_string()
    }
}

/// Render `builder` as a borderless, left-aligned table with a bold underlined header row.
pub(crate) fn render_table(builder: Builder) -> String {
    builder
        .build()
        .with(Style::blank())
        .with(
            Modify::new(Rows::first()).with(tabled::settings::Format::content(|value| {
                style(value).bold().underlined().to_string()
            })),
        )
        .with(Modify::new(Columns::new(..)).with(Alignment::left()))
        .to_string()
}

/// Print `builder` as a table, indented by two spaces.
pub(crate) fn print_table(builder: Builder) {
    for line in render_table(builder).lines() {
        println!("  {line}");
    }
}
