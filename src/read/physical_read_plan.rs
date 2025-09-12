use crate::read::physical_read_request::PhysicalReadRequest;
use crate::read::physical_read_results::PhysicalReadResultEntry;
use crate::read::physical_read_results::PhysicalReadResults;
use crate::windows::win_handles::AutoClosingHandle;
use std::ptr::null_mut;
use tracing::debug;
use tracing::warn;
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
    /// Adjust requests so each (offset,length) is 512-byte aligned by expanding outward.
    /// The logical offsets and lengths remain the same; we simply over-read and will trim later.
    pub fn align_512(&mut self) -> &mut Self {
        if self.requests.is_empty() {
            return self;
        }
        let sector: u64 = 512;
        for r in &mut self.requests {
            let phys = r.physical_offset.get::<byte>();
            let len = r.length.get::<byte>();
            let aligned_start = phys & !(sector - 1);
            let end = phys + len;
            let aligned_end = (end + sector - 1) & !(sector - 1);
            let new_len = aligned_end - aligned_start;
            if aligned_start != phys || new_len != len {
                r.length = Information::new::<byte>(new_len);
                r.physical_offset = Information::new::<byte>(aligned_start);
            }
        }
        // merging again may be beneficial
        self.merge_contiguous_reads();
        self
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

    pub fn read(self, filename: impl Param<PCWSTR>) -> eyre::Result<PhysicalReadResults> {
        use windows::Win32::Foundation::ERROR_IO_PENDING;
        const MAX_IN_FLIGHT_IO: usize = 32;

        if self.is_empty() {
            return Ok(PhysicalReadResults {
                entries: Vec::new(),
                total_size: Information::ZERO,
            });
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

            let requests = self.into_requests();
            let num = requests.len();
            let mut responses: Vec<Option<PhysicalReadResultEntry>> =
                (0..num).map(|_| None).collect();
            let mut in_flight = 0usize;
            let mut next_to_queue = 0usize;

            let mut queue_some = |in_flight: &mut usize| -> eyre::Result<()> {
                while *in_flight < MAX_IN_FLIGHT_IO && next_to_queue < num {
                    let req = &requests[next_to_queue];
                    let idx = next_to_queue;
                    let file_offset = req.physical_offset.get::<byte>();
                    let length = req.length.get::<byte>() as usize;
                    if file_offset & 0x1FF != 0 {
                        warn!(
                            "queuing unaligned physical_offset={} (not 512-byte aligned) length={length}",
                            file_offset
                        );
                    }
                    if length & 0x1FF != 0 {
                        warn!(
                            "queuing unaligned length={} (not 512-byte multiple) physical_offset={file_offset}",
                            length
                        );
                    }
                    let mut boxed = Box::new(ReadRequest {
                        overlapped: OVERLAPPED::default(),
                        buffer: vec![0u8; length],
                        file_offset,
                        length,
                        response_index: idx,
                        original: req.clone(),
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
                                return Err(eyre::eyre!(
                                    "ReadFile failed to queue (idx={idx} phys_offset={file_offset} len={length}): {e:?}"
                                ));
                            }
                        }
                    }
                    let _ = Box::into_raw(boxed);
                    *in_flight += 1;
                    next_to_queue += 1;
                }
                Ok(())
            };

            queue_some(&mut in_flight)?;
            let mut total_size: Information = Information::ZERO;

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
                        queue_some(&mut in_flight)?;
                        total_size += Information::new::<byte>(bytes_transferred.into());
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

            if next_to_queue != num {
                return Err(eyre::eyre!(
                    "Scheduler logic error after completion: queued {next_to_queue} of {num}"
                ));
            }
            let mut final_responses: Vec<PhysicalReadResultEntry> = responses
                .into_iter()
                .enumerate()
                .map(|(i, o)| o.ok_or_else(|| eyre::eyre!("Missing response index {i}")))
                .collect::<eyre::Result<_>>()?;
            final_responses.sort_by_key(|r| r.request.logical_offset.get::<byte>());

            Ok(PhysicalReadResults {
                entries: final_responses,
                total_size
            })
        }
    }
}
