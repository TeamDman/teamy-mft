use crate::query::Pathlike;
use color_eyre::owo_colors::OwoColorize;
use std::io;
use std::io::Write;
use std::ops::Deref;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, facet::Facet)]
pub struct QueryResultRow {
    pub path: Pathlike,
    pub has_deleted_entries: bool,
    pub is_ignored: bool,
}

unsafe impl vox_types::Reborrow for QueryResultRow {
    type Ref<'a> = QueryResultRow;
}

impl Deref for QueryResultRow {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.path
    }
}

impl QueryResultRow {
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
