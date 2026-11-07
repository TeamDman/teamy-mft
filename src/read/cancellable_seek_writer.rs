use std::io;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use teamy_cancellation::CancellableWriter;
use teamy_cancellation::CancellationScope;

/// Applies an operation's cancellation policy to both output and positioning.
#[derive(Debug)]
pub struct CancellableSeekWriter<'a, W: ?Sized> {
    writer: &'a mut W,
    cancel: &'a CancellationScope,
}

impl<'a, W: Write + Seek + ?Sized> CancellableSeekWriter<'a, W> {
    #[must_use]
    pub fn new(writer: &'a mut W, cancel: &'a CancellationScope) -> Self {
        Self { writer, cancel }
    }
}

impl<W: Write + ?Sized> Write for CancellableSeekWriter<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        CancellableWriter::new(&mut *self.writer, self.cancel).write(buf)
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        CancellableWriter::new(&mut *self.writer, self.cancel).write_all(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        CancellableWriter::new(&mut *self.writer, self.cancel).flush()
    }
}

impl<W: Seek + ?Sized> Seek for CancellableSeekWriter<'_, W> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.cancel.bail_if_cancelled().map_err(io::Error::other)?;
        self.writer.seek(pos)
    }
}

#[cfg(test)]
mod tests {
    use super::CancellableSeekWriter;
    use std::io;
    use std::io::Cursor;
    use std::io::Seek;
    use std::io::SeekFrom;
    use std::io::Write;
    use teamy_cancellation::CancellationError;
    use teamy_cancellation::CancellationToken;

    #[test]
    fn cancelled_scope_prevents_position_changes_writes_and_flushes() -> eyre::Result<()> {
        let token = CancellationToken::new();
        let mut output = Cursor::new(b"unchanged".to_vec());
        output.set_position(3);
        token.with_scope(|scope| -> eyre::Result<()> {
            token.request_cancel("stop output");
            let mut writer = CancellableSeekWriter::new(&mut output, &scope);
            for result in [
                writer.seek(SeekFrom::Start(0)).map(|_| ()),
                writer.write(b"changed").map(|_| ()),
                writer.write_all(b""),
                writer.flush(),
            ] {
                let error = result.unwrap_err();
                assert_eq!(error.kind(), io::ErrorKind::Other);
                assert_eq!(
                    error
                        .get_ref()
                        .unwrap()
                        .downcast_ref::<CancellationError>()
                        .unwrap()
                        .reason,
                    "stop output"
                );
            }
            Ok(())
        })?;
        assert_eq!(output.position(), 3);
        assert_eq!(output.into_inner(), b"unchanged");
        Ok(())
    }

    #[test]
    fn seeks_and_writes_preserve_underlying_cursor_semantics() -> eyre::Result<()> {
        let mut output = Cursor::new(b"abcdef".to_vec());
        CancellationToken::new().with_scope(|scope| -> eyre::Result<()> {
            let mut writer = CancellableSeekWriter::new(&mut output, &scope);
            assert_eq!(writer.seek(SeekFrom::Start(2))?, 2);
            writer.write_all(b"XY")?;
            assert_eq!(writer.seek(SeekFrom::End(-1))?, 5);
            assert_eq!(writer.write(b"!")?, 1);
            assert_eq!(writer.stream_position()?, 6);
            writer.flush()?;
            Ok(())
        })?;
        assert_eq!(output.position(), 6);
        assert_eq!(output.into_inner(), b"abXYe!");
        Ok(())
    }

    struct CancelDuringWrite {
        output: Cursor<Vec<u8>>,
        token: CancellationToken,
        writes: usize,
        largest_request: usize,
    }

    impl Write for CancelDuringWrite {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.writes += 1;
            self.largest_request = self.largest_request.max(buf.len());
            let written = self.output.write(buf)?;
            self.token.request_cancel("cancel during write");
            Ok(written)
        }

        fn flush(&mut self) -> io::Result<()> {
            self.output.flush()
        }
    }

    impl Seek for CancelDuringWrite {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
            self.output.seek(pos)
        }
    }

    #[test]
    fn cancellation_during_large_write_retains_only_the_bounded_prefix() -> eyre::Result<()> {
        let token = CancellationToken::new();
        let mut output = CancelDuringWrite {
            output: Cursor::new(Vec::new()),
            token: token.clone(),
            writes: 0,
            largest_request: 0,
        };
        token.with_scope(|scope| -> eyre::Result<()> {
            let mut writer = CancellableSeekWriter::new(&mut output, &scope);
            let error = writer.write_all(&vec![7; 200_000]).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::Other);
            assert_eq!(
                error
                    .get_ref()
                    .unwrap()
                    .downcast_ref::<CancellationError>()
                    .unwrap()
                    .reason,
                "cancel during write"
            );
            Ok(())
        })?;
        assert_eq!(output.writes, 1);
        assert_eq!(output.largest_request, 64 * 1024);
        assert_eq!(output.output.into_inner(), vec![7; 64 * 1024]);
        Ok(())
    }

    #[derive(Debug)]
    struct OriginalFailure;

    impl std::fmt::Display for OriginalFailure {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("underlying writer failure")
        }
    }

    impl std::error::Error for OriginalFailure {}

    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, OriginalFailure))
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                OriginalFailure,
            ))
        }
    }

    impl Seek for FailingWriter {
        fn seek(&mut self, _pos: SeekFrom) -> io::Result<u64> {
            Err(io::Error::new(io::ErrorKind::InvalidInput, OriginalFailure))
        }
    }

    #[test]
    fn underlying_errors_retain_their_kind_and_source() -> eyre::Result<()> {
        CancellationToken::new().with_scope(|scope| {
            let mut output = FailingWriter;
            let mut writer = CancellableSeekWriter::new(&mut output, &scope);
            for (error, kind) in [
                (writer.write(b"x").unwrap_err(), io::ErrorKind::BrokenPipe),
                (
                    writer.write_all(b"x").unwrap_err(),
                    io::ErrorKind::BrokenPipe,
                ),
                (writer.flush().unwrap_err(), io::ErrorKind::PermissionDenied),
                (
                    writer.seek(SeekFrom::Start(0)).unwrap_err(),
                    io::ErrorKind::InvalidInput,
                ),
            ] {
                assert_eq!(error.kind(), kind);
                assert!(error.get_ref().unwrap().is::<OriginalFailure>());
            }
            Ok(())
        })
    }
}
