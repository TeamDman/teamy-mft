use crate::domain::ParallelismLimit;
use crate::read::active_physical_read_request::ActivePhysicalReadRequest;
use crate::read::physical_read_request::PhysicalReadRequest;
use crate::read::physical_read_results::PhysicalReadResultEntry;
use crate::read::physical_read_results::PhysicalReadResults;
use eyre::Context;
use eyre::ContextCompat;
use std::collections::BTreeSet;
use std::num::NonZeroUsize;
use tracing::info_span;
use tracing::instrument;
use tracing::trace;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::CreateFileW;
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
use windows::Win32::Storage::FileSystem::FILE_FLAG_OVERLAPPED;
use windows::Win32::Storage::FileSystem::FILE_GENERIC_READ;
use windows::Win32::Storage::FileSystem::FILE_SHARE_DELETE;
use windows::Win32::Storage::FileSystem::FILE_SHARE_READ;
use windows::Win32::Storage::FileSystem::FILE_SHARE_WRITE;
use windows::Win32::Storage::FileSystem::OPEN_EXISTING;
use windows::Win32::System::IO::CancelIoEx;
use windows::Win32::System::IO::CreateIoCompletionPort;
use windows::core::Owned;
use windows::core::PCWSTR;
use windows::core::Param;

#[derive(Debug)]
pub struct PhysicalReader {
    remaining: Vec<PhysicalReadRequest>,
    results: Vec<Option<PhysicalReadResultEntry>>,
    in_flight: usize,
    max_in_flight: NonZeroUsize,
    /// File handle to read from
    file_handle: Owned<HANDLE>,
    /// IO Completion Port handle
    iocp_handle: Owned<HANDLE>,
}

#[derive(Debug)]
pub enum PhysicalReaderEnqueueResult {
    Enqueued,
    Full,
    Done,
}

impl PhysicalReader {
    const DEFAULT_MAX_IN_FLIGHT_IO: NonZeroUsize = NonZeroUsize::new(32).unwrap();

    pub(crate) fn configured_max_in_flight_io() -> eyre::Result<ParallelismLimit> {
        match std::env::var("TEAMY_MFT_MAX_IN_FLIGHT_IO") {
            Ok(value) => value.parse().wrap_err(
                "TEAMY_MFT_MAX_IN_FLIGHT_IO must be a non-negative integer (0: automatic)",
            ),
            Err(std::env::VarError::NotPresent) => Ok(ParallelismLimit::Automatic),
            Err(error) => Err(error).wrap_err("Failed reading TEAMY_MFT_MAX_IN_FLIGHT_IO"),
        }
    }

    /// Create a reader that queues overlapping reads against the specified file.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be opened or the completion port cannot be created.
    #[instrument(skip_all)]
    pub fn try_new(
        filename: impl Param<PCWSTR>,
        requests: impl IntoIterator<Item = PhysicalReadRequest>,
        max_in_flight: ParallelismLimit,
    ) -> eyre::Result<Self> {
        let max_in_flight = match max_in_flight {
            ParallelismLimit::Automatic => Self::DEFAULT_MAX_IN_FLIGHT_IO,
            ParallelismLimit::Fixed(limit) => limit,
        };
        // SAFETY: `CreateFileW` is called with valid path parameters and flags for overlapped IO.
        let file_handle = unsafe {
            Owned::new(CreateFileW(
                filename,
                FILE_GENERIC_READ.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OVERLAPPED,
                None,
            )?)
        };

        let mut remaining: Vec<PhysicalReadRequest> = requests.into_iter().collect();
        // pop() should schedule the lowest physical offsets first for sequential device I/O.
        remaining.sort_unstable();
        remaining.reverse();
        let results = (0..remaining.len()).map(|_| None).collect();
        // SAFETY: `CreateIoCompletionPort` is provided a valid handle and acceptable parameters.
        let completion_port =
            unsafe { Owned::new(CreateIoCompletionPort(*file_handle, None, 0, 0)?) };
        Ok(Self {
            remaining,
            results,
            in_flight: 0,
            max_in_flight,
            file_handle,
            iocp_handle: completion_port,
        })
    }

    /// Enqueue work until the IOCP queue has reached `max_in_flight` or there is no work left.
    ///
    /// # Errors
    ///
    /// Propagates errors returned by [`PhysicalReader::try_enqueue`].
    pub fn enqueue_until_saturation(&mut self) -> eyre::Result<()> {
        trace!(
            in_flight = self.in_flight,
            remaining = self.remaining.len(),
            max_in_flight = self.max_in_flight.get(),
            "Enqueuing IOCP reads",
        );
        while let PhysicalReaderEnqueueResult::Enqueued = self.try_enqueue()? {}
        Ok(())
    }

    /// Drain the queue and return the aggregated read results.
    ///
    /// # Errors
    ///
    /// Returns an error if queueing or completion handling fails.
    pub fn read_all(mut self) -> eyre::Result<PhysicalReadResults> {
        let _span = info_span!(
            "drain_physical_reader_iocp",
            request_count = self.remaining.len(),
            max_in_flight = self.max_in_flight.get(),
        )
        .entered();
        trace!(request_count = self.remaining.len(), "Queueing IOCP reads",);

        self.enqueue_until_saturation()?;

        trace!("Queue saturated, waiting for completions");
        while self.in_flight > 0 {
            self.receive_result()?;
            self.enqueue_until_saturation()?;
        }
        trace!("All IOCP reads completed");

        let entries = std::mem::take(&mut self.results)
            .into_iter()
            .enumerate()
            .map(|(i, o)| o.wrap_err_with(|| format!("Missing response index {i}")))
            .collect::<eyre::Result<BTreeSet<_>>>()?;
        Ok(PhysicalReadResults { entries })
    }

    /// Consume completed chunks immediately instead of retaining the entire source.
    ///
    /// # Errors
    ///
    /// Returns queue, short-read or consumer errors. Pending operations are cancelled and drained on drop.
    pub fn read_each(
        mut self,
        mut consume: impl FnMut(PhysicalReadResultEntry) -> eyre::Result<()>,
    ) -> eyre::Result<()> {
        self.enqueue_until_saturation()?;
        while self.in_flight > 0 {
            let result = ActivePhysicalReadRequest::receive(*self.iocp_handle);
            self.in_flight -= 1;
            let (entry, _) = result?;
            eyre::ensure!(
                entry.data.len() == entry.request.length.get::<uom::si::information::byte>(),
                "Short physical read"
            );
            consume(entry)?;
            self.enqueue_until_saturation()?;
        }
        Ok(())
    }

    /// Await a single IO completion result.
    ///
    /// # Errors
    ///
    /// Returns an error if waiting for the completion port fails.
    pub fn receive_result(&mut self) -> eyre::Result<()> {
        let result = ActivePhysicalReadRequest::receive(*self.iocp_handle);
        self.in_flight -= 1;
        match result {
            Ok((entry, response_index)) => {
                self.results[response_index] = Some(entry);
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    /// Attempt to enqueue the next read request if the queue is not saturated.
    ///
    /// # Errors
    ///
    /// Fails if sending the read request to the file handle fails.
    pub fn try_enqueue(&mut self) -> eyre::Result<PhysicalReaderEnqueueResult> {
        if self.in_flight >= self.max_in_flight.get() {
            return Ok(PhysicalReaderEnqueueResult::Full);
        }
        let Some(request) = self.remaining.pop() else {
            return Ok(PhysicalReaderEnqueueResult::Done);
        };

        let response_index = self.results.len() - self.remaining.len() - 1;
        let request = ActivePhysicalReadRequest::new(request, response_index);
        request
            .send(*self.file_handle)
            .wrap_err("Failed to send read request")?;
        self.in_flight += 1;
        trace!(
            in_flight = self.in_flight,
            remaining = self.remaining.len(),
            max_in_flight = self.max_in_flight.get(),
            "Enqueued IOCP read",
        );
        Ok(PhysicalReaderEnqueueResult::Enqueued)
    }
}

impl Drop for PhysicalReader {
    fn drop(&mut self) {
        if self.in_flight == 0 {
            return;
        }
        // SAFETY: the file handle belongs to this reader. Drain every completion before
        // closing its port so each leaked request allocation is reclaimed exactly once.
        let _ = unsafe { CancelIoEx(*self.file_handle, None) };
        while self.in_flight > 0 {
            let _ = ActivePhysicalReadRequest::receive(*self.iocp_handle);
            self.in_flight -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PhysicalReader;
    use crate::domain::ParallelismLimit;
    use crate::windows_utils::string::EasyPCWSTR;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::process::Command;

    #[test]
    fn configured_max_in_flight_io_parses_environment() -> eyre::Result<()> {
        const EXPECTED: &str = "TEAMY_MFT_TEST_EXPECTED_IO_LIMIT";
        if let Ok(expected) = std::env::var(EXPECTED) {
            let parsed = PhysicalReader::configured_max_in_flight_io();
            if expected == "error" {
                assert!(
                    parsed
                        .unwrap_err()
                        .to_string()
                        .contains("TEAMY_MFT_MAX_IN_FLIGHT_IO")
                );
            } else {
                assert_eq!(usize::from(parsed?), expected.parse::<usize>()?);
            }
            return Ok(());
        }

        for (value, expected) in [
            (None, "0"),
            (Some(OsString::from("0")), "0"),
            (Some(OsString::from("1")), "1"),
            (Some(OsString::from("64")), "64"),
            (Some(OsString::from("")), "error"),
            (Some(OsString::from("invalid")), "error"),
            (Some(OsString::from("-1")), "error"),
            (Some(OsString::from(u128::MAX.to_string())), "error"),
            (Some(OsString::from_wide(&[0xd800])), "error"),
        ] {
            let mut child = Command::new(std::env::current_exe()?);
            child
                .args([
                    "--exact",
                    "read::physical_reader::tests::configured_max_in_flight_io_parses_environment",
                    "--nocapture",
                ])
                .env(EXPECTED, expected)
                .env_remove("TEAMY_MFT_MAX_IN_FLIGHT_IO");
            if let Some(value) = &value {
                child.env("TEAMY_MFT_MAX_IN_FLIGHT_IO", value);
            }
            let output = child.output()?;
            assert!(
                output.status.success(),
                "configuration {value:?} failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
        }
        Ok(())
    }

    #[test]
    fn reader_resolves_automatic_and_fixed_limits() -> eyre::Result<()> {
        let source = tempfile::NamedTempFile::new()?;
        let path = source.path().easy_pcwstr()?;
        for (limit, expected) in [
            (ParallelismLimit::Automatic, 32),
            (ParallelismLimit::from(7usize), 7),
        ] {
            let reader = PhysicalReader::try_new(&path, std::iter::empty(), limit)?;
            assert_eq!(reader.max_in_flight.get(), expected);
        }
        Ok(())
    }
}
