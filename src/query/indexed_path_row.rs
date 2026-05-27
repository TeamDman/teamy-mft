use crate::query::QueryResultPath;
use color_eyre::owo_colors::OwoColorize;
use std::io;
use std::io::Write;

#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct IndexedPathRow {
    pub path: QueryResultPath,
    pub has_deleted_entries: bool,
    pub is_ignored: bool,
}

impl IndexedPathRow {
    pub fn render_path<W>(&self, writer: &mut W, colorize: bool) -> io::Result<()>
    where
        W: Write,
    {
        if !colorize {
            return write!(writer, "{}", self.path);
        }
        if self.is_ignored {
            return write!(writer, "{}", self.path.as_str().yellow());
        }
        if self.has_deleted_entries {
            write!(writer, "{}", self.path.as_str().red())
        } else {
            write!(writer, "{}", self.path.as_str().green())
        }
    }
}
