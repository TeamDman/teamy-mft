use std::io::Write;
use std::ops::Deref;
use std::ops::DerefMut;
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::Mutex;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::fmt::writer::BoxMakeWriter;
use tracing_subscriber::fmt::writer::Tee;

/// Captures logs to be replayed later when the user requests to see them.
///
/// ```
/// use teamy_mft::windows_utils::log::LOG_BUFFER;
/// use tracing::Level;
/// use tracing_subscriber::fmt::SubscriberBuilder;
/// use tracing_subscriber::fmt::writer::MakeWriterExt;
/// use tracing_subscriber::util::SubscriberInitExt;
/// SubscriberBuilder::default()
///    .with_writer(std::io::stderr.and(LOG_BUFFER.clone()))
///    .finish()
///    .init();
/// ```
pub static LOG_BUFFER: LazyLock<BufferSink> = LazyLock::new(BufferSink::default);

pub static DUAL_WRITER: LazyLock<Tee<BoxMakeWriter, BufferSink>> =
    LazyLock::new(|| Tee::new(BoxMakeWriter::new(std::io::stderr), LOG_BUFFER.clone()));

/// Logs are stored in a buffer to be displayed in the console when the user clicks show logs
#[derive(Debug, Clone, Default)]
pub struct BufferSink {
    buffer: Arc<Mutex<Vec<u8>>>,
}
impl BufferSink {
    pub fn replay(&self, writer: &mut impl Write) -> eyre::Result<()> {
        let buffer = self.lock().unwrap();
        writeln!(writer, "=== Previous Logs ===")?;
        writer
            .write_all(&buffer)
            .map_err(|e| eyre::eyre!("Failed to write log buffer to writer: {}", e))?;
        writeln!(writer, "=== End of Previous Logs ===")?;
        Ok(())
    }
}
impl Deref for BufferSink {
    type Target = Arc<Mutex<Vec<u8>>>;

    fn deref(&self) -> &Self::Target {
        &self.buffer
    }
}
impl DerefMut for BufferSink {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.buffer
    }
}
impl Write for BufferSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut buffer = self.lock().unwrap();
        buffer.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl<'a> MakeWriter<'a> for BufferSink {
    type Writer = BufferSink;

    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}
