use std::io;
use std::io::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResultListPresentation {
    pub available_width: usize,
    pub column_gap: usize,
}

impl ResultListPresentation {
    #[must_use]
    pub fn for_terminal() -> Self {
        Self {
            available_width: crate::windows_utils::console::terminal_size()
                .map(|(columns, _)| columns)
                .filter(|value| *value > 0)
                .or_else(|| {
                    std::env::var("COLUMNS")
                        .ok()
                        .and_then(|value| value.parse::<usize>().ok())
                        .filter(|value| *value > 0)
                })
                .unwrap_or(120usize),
            column_gap: 2,
        }
    }

    /// # Errors
    ///
    /// Returns an error if rendering a row or writing to `writer` fails.
    pub fn write_result_list<T, W, Width, Render>(
        &self,
        rows: &[T],
        writer: &mut W,
        use_columns: bool,
        display_width: Width,
        render: Render,
    ) -> io::Result<()>
    where
        W: Write,
        Width: Fn(&T) -> usize,
        Render: Fn(&T, &mut W) -> io::Result<()>,
    {
        if use_columns {
            self.write_columns(rows, writer, display_width, render)
        } else {
            Self::write_lines(rows, writer, render)
        }
    }

    fn write_lines<T, W, Render>(rows: &[T], writer: &mut W, render: Render) -> io::Result<()>
    where
        W: Write,
        Render: Fn(&T, &mut W) -> io::Result<()>,
    {
        for row in rows {
            render(row, writer)?;
            writeln!(writer)?;
        }

        Ok(())
    }

    fn write_columns<T, W, Width, Render>(
        &self,
        rows: &[T],
        writer: &mut W,
        display_width: Width,
        render: Render,
    ) -> io::Result<()>
    where
        W: Write,
        Width: Fn(&T) -> usize,
        Render: Fn(&T, &mut W) -> io::Result<()>,
    {
        if rows.is_empty() {
            return Ok(());
        }

        let max_width = rows.iter().map(&display_width).max().unwrap_or(1).max(1);
        let column_count =
            ((self.available_width + self.column_gap) / (max_width + self.column_gap)).max(1);
        let row_count = rows.len().div_ceil(column_count);

        for row_index in 0..row_count {
            for column_index in 0..column_count {
                let index = row_index + column_index * row_count;
                if index >= rows.len() {
                    continue;
                }

                let row = &rows[index];
                render(row, writer)?;

                if column_index + 1 < column_count {
                    let pad = (max_width + self.column_gap).saturating_sub(display_width(row));
                    write!(writer, "{}", " ".repeat(pad))?;
                }
            }

            writeln!(writer)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::ResultListPresentation;
    use std::io::Write;

    #[test]
    fn writes_one_result_per_line_when_columns_are_disabled() -> std::io::Result<()> {
        let rows = ["alpha", "beta"];
        let mut output = Vec::new();

        ResultListPresentation {
            available_width: 80,
            column_gap: 2,
        }
        .write_result_list(
            &rows,
            &mut output,
            false,
            |row| row.chars().count(),
            |row, writer| write!(writer, "{row}"),
        )?;

        assert_eq!(String::from_utf8(output).unwrap(), "alpha\nbeta\n");
        Ok(())
    }

    #[test]
    fn writes_column_major_columns_when_enabled() -> std::io::Result<()> {
        let rows = ["aa", "bb", "cc", "dd"];
        let mut output = Vec::new();

        ResultListPresentation {
            available_width: 8,
            column_gap: 2,
        }
        .write_result_list(
            &rows,
            &mut output,
            true,
            |row| row.chars().count(),
            |row, writer| write!(writer, "{row}"),
        )?;

        assert_eq!(String::from_utf8(output).unwrap(), "aa  cc\nbb  dd\n");
        Ok(())
    }
}
