use crate::windows::win_handles::AutoClosingHandle;
use std::ptr::null_mut;
use tracing::debug;
use uom::ConstZero;
use uom::si::information::byte;
use uom::si::u64::Information;
use windows::Win32::Storage::FileSystem::CreateFileW;
use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_NORMAL;
use windows::Win32::Storage::FileSystem::FILE_FLAG_OVERLAPPED;
use windows::Win32::Storage::FileSystem::FILE_GENERIC_READ;
use windows::Win32::Storage::FileSystem::FILE_SHARE_DELETE;
use windows::Win32::Storage::FileSystem::FILE_SHARE_READ;
use windows::Win32::Storage::FileSystem::FILE_SHARE_WRITE;
use windows::Win32::Storage::FileSystem::OPEN_EXISTING;
use windows::Win32::Storage::FileSystem::ReadFile;
use windows::Win32::System::IO::CreateIoCompletionPort;
use windows::Win32::System::IO::GetQueuedCompletionStatus;
use windows::Win32::System::IO::OVERLAPPED;
use windows::core::PCWSTR;
use windows::core::Param;

// ---------------- Physical Read Plan ----------------
#[derive(Debug, Clone)]
pub struct PhysicalReadRequest {
    pub physical_offset: Information,
    pub logical_offset: Information,
    pub length: Information,
}
impl PhysicalReadRequest {
    pub fn new(
        physical_offset: Information,
        logical_offset: Information,
        length: Information,
    ) -> Self {
        Self {
            physical_offset,
            logical_offset,
            length,
        }
    }
    pub fn physical_end(&self) -> Information {
        self.physical_offset + self.length
    }
    pub fn logical_end(&self) -> Information {
        self.logical_offset + self.length
    }
}

#[derive(Debug, Clone)]
pub struct PhysicalReadResultEntry {
    pub request: PhysicalReadRequest,
    pub data: Vec<u8>,
}

#[derive(Debug, Default, Clone)]
pub struct PhysicalReadPlan {
    requests: Vec<PhysicalReadRequest>,
    total_requested: Information,
}
impl PhysicalReadPlan {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(
        &mut self,
        physical_offset: Information,
        logical_offset: Information,
        length: Information,
    ) {
        if length == Information::ZERO {
            return;
        }
        self.requests.push(PhysicalReadRequest::new(
            physical_offset,
            logical_offset,
            length,
        ));
        self.total_requested += length;
    }
    /// Merge physically & logically contiguous requests (safe default). Returns &mut self for chaining.
    pub fn merge_contiguous_reads(&mut self) -> &mut Self {
        if self.requests.is_empty() {
            return self;
        }
        self.requests.sort_by_key(|r| {
            (
                r.physical_offset.get::<byte>(),
                r.logical_offset.get::<byte>(),
            )
        });
        let mut merged: Vec<PhysicalReadRequest> = Vec::with_capacity(self.requests.len());
        for req in self.requests.drain(..) {
            if let Some(last) = merged.last_mut()
                && last.physical_end() == req.physical_offset
                    && last.logical_end() == req.logical_offset
                {
                    last.length += req.length;
                    continue;
                }
            merged.push(req);
        }
        self.requests = merged;
        self
    }
    /// Split requests into uniform <= chunk_size pieces. Returns a new plan.
    pub fn chunked(&self, chunk_size: Information) -> Self {
        if chunk_size == Information::ZERO {
            return self.clone();
        }
        let mut out = PhysicalReadPlan::new();
        let sz = chunk_size.get::<byte>();
        for req in &self.requests {
            let mut remaining = req.length.get::<byte>();
            let mut phys = req.physical_offset.get::<byte>();
            let mut log = req.logical_offset.get::<byte>();
            while remaining > 0 {
                let this = if remaining > sz { sz } else { remaining };
                out.push(
                    Information::new::<byte>(phys),
                    Information::new::<byte>(log),
                    Information::new::<byte>(this),
                );
                phys += this;
                log += this;
                remaining -= this;
            }
        }
        out
    }
    pub fn is_empty(&self) -> bool {
        self.requests.is_empty()
    }
    pub fn num_requests(&self) -> usize {
        self.requests.len()
    }
    pub fn total_requested_information(&self) -> Information {
        self.total_requested
    }
    pub fn requests(&self) -> &[PhysicalReadRequest] {
        &self.requests
    }
    pub fn into_requests(self) -> Vec<PhysicalReadRequest> {
        self.requests
    }

    pub fn read(self, filename: impl Param<PCWSTR>) -> eyre::Result<Vec<PhysicalReadResultEntry>> {
        use windows::Win32::Foundation::ERROR_IO_PENDING;
        const MAX_IN_FLIGHT_IO: usize = 32;

        if self.is_empty() {
            return Ok(Vec::new());
        }

        #[repr(C)]
        struct ReadRequest {
            overlapped: OVERLAPPED,
            buffer: Vec<u8>,
            file_offset: u64,
            length: usize,
            response_index: usize,
            original: PhysicalReadRequest,
        }

        unsafe {
            let handle: AutoClosingHandle = CreateFileW(
                filename,
                FILE_GENERIC_READ.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OVERLAPPED,
                None,
            )?
            .into();

            let completion_port: AutoClosingHandle =
                CreateIoCompletionPort(*handle, None, 0, 0)?.into();

            debug!(
                "Queueing {} IOCP reads ({} total)",
                self.num_requests(),
                self.total_requested_information().get_human()
            );

            let num = self.num_requests();
            // Use with_capacity + push to avoid requiring Clone on Option<PhysicalReadResponse>
            let mut responses: Vec<Option<PhysicalReadResultEntry>> =
                (0..num).map(|_| None).collect();
            let mut in_flight = 0usize;
            let mut iter = self.into_requests().into_iter().enumerate();

            let mut post_more = |in_flight: &mut usize| -> eyre::Result<()> {
                for (idx, req) in &mut iter {
                    if *in_flight >= MAX_IN_FLIGHT_IO {
                        break;
                    }
                    let file_offset = req.physical_offset.get::<byte>();
                    let length = req.length.get::<byte>() as usize;
                    let mut boxed = Box::new(ReadRequest {
                        overlapped: OVERLAPPED::default(),
                        buffer: vec![0u8; length],
                        file_offset,
                        length,
                        response_index: idx,
                        original: req,
                    });
                    boxed.overlapped.Anonymous.Anonymous.Offset =
                        (file_offset & 0xFFFF_FFFF) as u32;
                    boxed.overlapped.Anonymous.Anonymous.OffsetHigh =
                        ((file_offset >> 32) & 0xFFFF_FFFF) as u32;
                    let overlapped_ptr: *mut OVERLAPPED = &mut boxed.overlapped;
                    match ReadFile(
                        *handle,
                        Some(&mut boxed.buffer[..]),
                        None,
                        Some(overlapped_ptr),
                    ) {
                        Ok(()) => {}
                        Err(e) => {
                            if e.code() != ERROR_IO_PENDING.into() {
                                return Err(eyre::eyre!("ReadFile failed to queue: {e:?}"));
                            }
                        }
                    }
                    let _ = Box::into_raw(boxed);
                    *in_flight += 1;
                }
                Ok(())
            };

            post_more(&mut in_flight)?;

            while in_flight > 0 {
                let mut bytes_transferred: u32 = 0;
                let mut completion_key: usize = 0;
                let mut lp_overlapped: *mut OVERLAPPED = null_mut();
                let res = GetQueuedCompletionStatus(
                    *completion_port,
                    &mut bytes_transferred as *mut u32,
                    &mut completion_key as *mut usize,
                    &mut lp_overlapped as *mut *mut OVERLAPPED,
                    u32::MAX,
                );
                match res {
                    Ok(()) => {
                        if lp_overlapped.is_null() {
                            return Err(eyre::eyre!(
                                "IOCP returned success but OVERLAPPED ptr was null"
                            ));
                        }
                        let req_ptr = lp_overlapped as *mut ReadRequest;
                        let boxed_req = Box::from_raw(req_ptr);
                        let mut data = boxed_req.buffer;
                        let copy_len = (bytes_transferred as usize).min(boxed_req.length);
                        if copy_len < data.len() {
                            data.truncate(copy_len);
                        }
                        responses[boxed_req.response_index] = Some(PhysicalReadResultEntry {
                            request: boxed_req.original,
                            data,
                        });
                        in_flight -= 1;
                        post_more(&mut in_flight)?;
                    }
                    Err(e) => {
                        if lp_overlapped.is_null() {
                            return Err(eyre::eyre!("GetQueuedCompletionStatus failed: {e:?}"));
                        } else {
                            let req_ptr = lp_overlapped as *mut ReadRequest;
                            let boxed_req = Box::from_raw(req_ptr);
                            return Err(eyre::eyre!(
                                "I/O operation failed for offset {} length {}: {e:?}",
                                boxed_req.file_offset,
                                boxed_req.length
                            ));
                        }
                    }
                }
            }

            let mut final_responses: Vec<PhysicalReadResultEntry> = responses
                .into_iter()
                .enumerate()
                .map(|(i, o)| o.ok_or_else(|| eyre::eyre!("Missing response index {i}")))
                .collect::<eyre::Result<_>>()?;
            final_responses.sort_by_key(|r| r.request.logical_offset.get::<byte>());

            Ok(final_responses)
        }
    }
}

// --------------- Convenience Helpers ---------------
impl PhysicalReadResultEntry {
    /// Logical start offset in bytes as u64
    pub fn logical_offset_bytes(&self) -> u64 {
        self.request.logical_offset.get::<byte>()
    }
    /// Length in bytes as u64
    pub fn length_bytes(&self) -> u64 {
        self.request.length.get::<byte>()
    }
}

/// Write the collected physical read results into a file at the provided path.
/// Assumes results may be out-of-order; sorts by logical offset, pre-sizes the file to `total_logical_size`
/// (zeros for gaps), then writes each block at its logical offset.
pub fn write_read_results_to_file(
    mut entries: Vec<PhysicalReadResultEntry>,
    output_path: impl AsRef<std::path::Path>,
    total_logical_size: u64,
) -> eyre::Result<()> {
    use std::io::Seek;
    use std::io::SeekFrom;
    use std::io::Write;
    if entries.is_empty() {
        // still create & size (may represent all-sparse data)
        let file = std::fs::File::create(output_path)?;
        file.set_len(total_logical_size)?;
        return Ok(());
    }
    entries.sort_by_key(|e| e.logical_offset_bytes());
    let file = std::fs::File::create(output_path)?;
    file.set_len(total_logical_size)?;
    let mut writer = std::io::BufWriter::new(file);
    for e in entries {
        writer.seek(SeekFrom::Start(e.logical_offset_bytes()))?;
        writer.write_all(&e.data)?;
    }
    writer.flush()?;
    Ok(())
}
