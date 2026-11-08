use crate::domain::ParallelismLimit;
use crate::read::active_physical_read_request::ActivePhysicalReadRequest;
use crate::read::logical_read_plan::LogicalReadPlan;
use crate::read::physical_read_plan::PhysicalReadPlan;
use crate::read::physical_read_request::PhysicalReadRequest;
use crate::read::physical_read_result_entry::PhysicalReadResultEntry;
use eyre::Context;
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
pub struct PhysicalReader<'plan> {
    logical_read_plan: &'plan LogicalReadPlan,
    remaining: Vec<PhysicalReadRequest>,
    in_flight: usize,
    max_in_flight: NonZeroUsize,
    /// File handle to read from
    file_handle: Owned<HANDLE>,
    /// IO Completion Port handle
    iocp_handle: Owned<HANDLE>,
}

impl<'plan> PhysicalReader<'plan> {
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
        physical_read_plan: PhysicalReadPlan<'plan>,
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

        let logical_read_plan = physical_read_plan.logical_plan();
        // pop() should schedule the lowest physical offsets first for sequential device I/O.
        let remaining = physical_read_plan.into_iter().rev().collect();
        // SAFETY: `CreateIoCompletionPort` is provided a valid handle and acceptable parameters.
        let completion_port =
            unsafe { Owned::new(CreateIoCompletionPort(*file_handle, None, 0, 0)?) };
        Ok(Self {
            logical_read_plan,
            remaining,
            in_flight: 0,
            max_in_flight,
            file_handle,
            iocp_handle: completion_port,
        })
    }

    /// Consume completed chunks immediately instead of retaining the entire source.
    ///
    /// # Errors
    ///
    /// Returns queue, short-read or consumer errors. Pending operations are cancelled and drained on drop.
    pub fn read_each(
        mut self,
        mut consume: impl FnMut(PhysicalReadResultEntry<'plan>) -> eyre::Result<()>,
    ) -> eyre::Result<()> {
        let _span = info_span!(
            "drain_physical_reader_iocp",
            request_count = self.remaining.len(),
            max_in_flight = self.max_in_flight.get(),
        )
        .entered();
        loop {
            while self.in_flight < self.max_in_flight.get() {
                let Some(request) = self.remaining.pop() else {
                    break;
                };
                ActivePhysicalReadRequest::new(request)
                    .send(*self.file_handle)
                    .wrap_err("Failed to send read request")?;
                self.in_flight += 1;
                trace!(
                    in_flight = self.in_flight,
                    remaining = self.remaining.len(),
                    max_in_flight = self.max_in_flight.get(),
                    "Enqueued IOCP read",
                );
            }
            if self.in_flight == 0 {
                break;
            }
            // SAFETY: This reader exclusively owns the live port. Only its submitted
            // requests target it, no foreign packets are posted, and each received
            // completion reclaims its request allocation exactly once.
            let result = unsafe { ActivePhysicalReadRequest::receive(*self.iocp_handle) };
            self.in_flight -= 1;
            let (request, data) = result?;
            consume(PhysicalReadResultEntry::try_new(
                self.logical_read_plan,
                request,
                data,
            )?)?;
        }
        trace!("All IOCP reads completed");
        Ok(())
    }
}

impl Drop for PhysicalReader<'_> {
    fn drop(&mut self) {
        if self.in_flight == 0 {
            return;
        }
        // SAFETY: the file handle belongs to this reader. Drain every completion before
        // closing its port so each leaked request allocation is reclaimed exactly once.
        let _ = unsafe { CancelIoEx(*self.file_handle, None) };
        while self.in_flight > 0 {
            // SAFETY: We exclusively own the live port and drain only our remaining
            // submitted requests. Previously received allocations are already gone;
            // each remaining completion reclaims its allocation once before closing.
            let _ = unsafe { ActivePhysicalReadRequest::receive(*self.iocp_handle) };
            self.in_flight -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PhysicalReader;
    use crate::domain::ParallelismLimit;
    use crate::read::logical_read_plan::LogicalReadPlan;
    use crate::read::physical_read_plan::PhysicalReadPlan;
    use crate::windows_utils::string::EasyPCWSTR;
    use std::collections::BTreeSet;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::process::Command;

    #[test]
    #[expect(
        clippy::disallowed_methods,
        reason = "A child test process isolates environment variables without racing other tests; no service management is performed."
    )]
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
        let logical = LogicalReadPlan {
            segments: BTreeSet::new(),
        };
        for (limit, expected) in [
            (ParallelismLimit::Automatic, 32),
            (ParallelismLimit::from(7usize), 7),
        ] {
            let reader =
                PhysicalReader::try_new(&path, PhysicalReadPlan::try_from(&logical)?, limit)?;
            assert_eq!(reader.max_in_flight.get(), expected);
        }
        Ok(())
    }
}
