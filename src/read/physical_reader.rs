use crate::read::active_physical_read_request::ActivePhysicalReadRequest;
use crate::read::physical_read_request::PhysicalReadRequest;
use crate::read::physical_read_results::PhysicalReadResultEntry;
use crate::read::physical_read_results::PhysicalReadResults;
use eyre::Context;
use std::collections::BTreeSet;
use tracing::debug;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::CreateFileW;
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
use windows::Win32::Storage::FileSystem::FILE_FLAG_OVERLAPPED;
use windows::Win32::Storage::FileSystem::FILE_GENERIC_READ;
use windows::Win32::Storage::FileSystem::FILE_SHARE_DELETE;
use windows::Win32::Storage::FileSystem::FILE_SHARE_READ;
use windows::Win32::Storage::FileSystem::FILE_SHARE_WRITE;
use windows::Win32::Storage::FileSystem::OPEN_EXISTING;
use windows::Win32::System::IO::CreateIoCompletionPort;
use windows::core::Owned;
use windows::core::PCWSTR;
use windows::core::Param;

pub struct PhysicalReader {
    remaining: Vec<PhysicalReadRequest>,
    results: Vec<Option<PhysicalReadResultEntry>>,
    in_flight: usize,
    max_in_flight: usize,
    /// File handle to read from
    file_handle: Owned<HANDLE>,
    /// IO Completion Port handle
    iocp_handle: Owned<HANDLE>,
}

pub enum PhysicalReaderEnqueueResult {
    Enqueued,
    Full,
    Done,
}

impl PhysicalReader {
    pub fn try_new(
        filename: impl Param<PCWSTR>,
        requests: impl IntoIterator<Item = PhysicalReadRequest>,
        max_in_flight: usize,
    ) -> eyre::Result<Self> {
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

        let remaining: Vec<PhysicalReadRequest> = requests.into_iter().collect();
        let results = (0..remaining.len()).map(|_| None).collect();
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

    pub fn enqueue_until_saturation(&mut self) -> eyre::Result<()> {
        debug!(
            in_flight = self.in_flight,
            remaining = self.remaining.len(),
            max_in_flight = self.max_in_flight,
            "Enqueuing IOCP reads",
        );
        loop {
            match self.try_enqueue()? {
                PhysicalReaderEnqueueResult::Enqueued => {}
                PhysicalReaderEnqueueResult::Full => break,
                PhysicalReaderEnqueueResult::Done => break,
            }
        }
        Ok(())
    }

    pub fn read_all(mut self) -> eyre::Result<PhysicalReadResults> {
        debug!(request_count = self.remaining.len(), "Queueing IOCP reads",);

        self.enqueue_until_saturation()?;

        debug!("Queue saturated, waiting for completions");
        while self.in_flight > 0 {
            self.receive_result()?;
            self.enqueue_until_saturation()?;
        }
        debug!("All IOCP reads completed");

        let entries = self
            .results
            .into_iter()
            .enumerate()
            .map(|(i, o)| o.ok_or_else(|| eyre::eyre!("Missing response index {i}")))
            .collect::<eyre::Result<BTreeSet<_>>>()?;
        Ok(PhysicalReadResults { entries })
    }

    pub fn receive_result(&mut self) -> eyre::Result<()> {
        match ActivePhysicalReadRequest::receive(*self.iocp_handle) {
            Ok((entry, response_index)) => {
                self.results[response_index] = Some(entry);
                self.in_flight -= 1;
                Ok(())
            }
            Err(e) => Err(e),
        }
    }

    pub fn try_enqueue(&mut self) -> eyre::Result<PhysicalReaderEnqueueResult> {
        if self.in_flight >= self.max_in_flight {
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
        debug!(
            in_flight = self.in_flight,
            remaining = self.remaining.len(),
            max_in_flight = self.max_in_flight,
            "Enqueued IOCP read",
        );
        Ok(PhysicalReaderEnqueueResult::Enqueued)
    }
}
