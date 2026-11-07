use crate::read::physical_read_request::PhysicalReadRequest;
use eyre::WrapErr;
use eyre::bail;
use std::any::type_name;
use std::ptr::null_mut;
use tracing::trace;
use tracing::warn;
use uom::si::information::byte;
use windows::Win32::Foundation::ERROR_IO_PENDING;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::ReadFile;
use windows::Win32::System::IO::GetQueuedCompletionStatus;
use windows::Win32::System::IO::OVERLAPPED;

// `overlapped` must remain first under `repr(C)`: its completion pointer identifies
// the entire allocation released by `Box::into_raw` and reclaimed by `receive`.
#[repr(C)]
pub(super) struct ActivePhysicalReadRequest {
    overlapped: OVERLAPPED,
    buffer: Vec<u8>,
    original: PhysicalReadRequest,
}
impl std::fmt::Debug for ActivePhysicalReadRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct(type_name::<ActivePhysicalReadRequest>())
            .field("file_offset", &self.original.offset)
            .field("length", &self.original.length)
            .field("buffer length", &self.buffer.len())
            .field("overlapped", &format_args!("{:p}", &self.overlapped))
            .finish_non_exhaustive()
    }
}

impl ActivePhysicalReadRequest {
    /// Prepares the overlapped read request with the supplied payload and target.
    ///
    /// # Panics
    ///
    /// Panics if the masked offset slices do not fit within `u32`, which should
    /// never happen on supported target pointer widths.
    pub(super) fn new(request: PhysicalReadRequest) -> Self {
        let overlapped = {
            let mut overlapped = OVERLAPPED::default();
            let offset = request.offset.get::<byte>();
            if request.offset.get::<byte>() & 0x1FF != 0 {
                warn!(
                    ?request.offset,
                    ?request.length,
                    "Constructing {} - not 512-byte aligned",
                    type_name::<PhysicalReadRequest>(),
                );
            }
            if request.length.get::<byte>() & 0x1FF != 0 {
                warn!(
                    ?request.offset,
                    ?request.length,
                    "Constructing {} - not 512-byte multiple",
                    type_name::<PhysicalReadRequest>(),
                );
            }
            let offset_low =
                u32::try_from(offset & 0xFFFF_FFFF).expect("offset low bits fit into u32");
            let offset_high =
                u32::try_from((offset >> 32) & 0xFFFF_FFFF).expect("offset high bits fit into u32");
            overlapped.Anonymous.Anonymous.Offset = offset_low;
            overlapped.Anonymous.Anonymous.OffsetHigh = offset_high;
            overlapped
        };
        Self {
            overlapped,
            buffer: vec![0; request.length.get::<byte>()],
            original: request,
        }
    }

    /// Queues this request for asynchronous completion on `file_handle`.
    ///
    /// # Errors
    ///
    /// Returns an error if the buffer exceeds Win32's byte-count limit or
    /// `ReadFile` rejects the request for a reason other than `ERROR_IO_PENDING`.
    pub(super) fn send(self, file_handle: HANDLE) -> eyre::Result<()> {
        // The projected ReadFile API unwraps its usize-to-u32 conversion.
        u32::try_from(self.buffer.len())
            .wrap_err("Physical read buffer exceeds Win32's u32 byte-count limit")?;
        let request_ptr = Box::into_raw(Box::new(self));

        // SAFETY: both pointers project disjoint fields of the live raw allocation.
        // No Box or reference to the entire request is recreated while I/O is pending.
        // The allocation and buffer stay alive until rejection or dequeued completion.
        match unsafe {
            let overlapped_ptr = &raw mut (*request_ptr).overlapped;
            let buffer = (&mut (*request_ptr).buffer).as_mut_slice();
            ReadFile(file_handle, Some(buffer), None, Some(overlapped_ptr))
        } {
            Ok(()) => {}
            Err(e) => {
                if e.code() != ERROR_IO_PENDING.into() {
                    // SAFETY: the OS rejected submission, so it retains neither pointer
                    // and no completion can reclaim this allocation.
                    let boxed = unsafe { Box::from_raw(request_ptr) };
                    bail!("ReadFile failed to queue request {boxed:?}: {e:?}");
                }
            }
        }

        // Accepted requests are owned by the completion path; do not access request_ptr again.
        Ok(())
    }

    /// Waits for the next completion from `completion_port` and returns the
    /// original request and complete read payload.
    ///
    /// # Errors
    ///
    /// Returns an error if IOCP fails, the completion is short, or a completion
    /// has a null overlapped pointer (which violates the request invariants).
    ///
    /// # Safety
    ///
    /// The port must remain open and contain only completions for requests accepted
    /// by `Self::send`, with their original `OVERLAPPED` pointers. The corresponding
    /// `send` call must have returned before receiving its completion. Foreign or
    /// posted packets are forbidden. Each allocation must be reclaimed exactly once,
    /// through its completion, and remain untouched by Rust code until then.
    pub(super) unsafe fn receive(
        completion_port: HANDLE,
    ) -> eyre::Result<(PhysicalReadRequest, Vec<u8>)> {
        let mut bytes_transferred: u32 = 0;
        let mut completion_key: usize = 0;
        let mut lp_overlapped: *mut OVERLAPPED = null_mut();
        trace!("Waiting for IOCP read completion");
        // SAFETY: The raw pointers remain valid for the duration of the
        // call and are initialized before the call returns.
        let res = unsafe {
            GetQueuedCompletionStatus(
                completion_port,
                &raw mut bytes_transferred,
                &raw mut completion_key,
                &raw mut lp_overlapped,
                u32::MAX,
            )
        };
        if lp_overlapped.is_null() {
            res.wrap_err("GetQueuedCompletionStatus failed without dequeuing a completion")?;
            bail!("IOCP returned success but OVERLAPPED ptr was null");
        }
        // SAFETY: the caller guarantees this is an unreclaimed allocation from send.
        // repr(C) and the first-field layout recover its original pointer. The operation
        // has completed, so neither the OS nor another receiver may use the allocation.
        let boxed_req = unsafe { Box::from_raw(lp_overlapped.cast::<Self>()) };
        res.wrap_err_with(|| format!("I/O operation failed for {boxed_req:?}"))?;
        trace!(?boxed_req, bytes_transferred, "Completed IOCP read",);
        let expected_len = boxed_req.original.length.get::<byte>();
        eyre::ensure!(
            usize::try_from(bytes_transferred)? == expected_len
                && boxed_req.buffer.len() == expected_len,
            "Short physical read: expected {expected_len} bytes, received {bytes_transferred}"
        );
        Ok((boxed_req.original, boxed_req.buffer))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::windows_utils::string::EasyPCWSTR;
    use std::ptr::from_mut;
    use uom::si::usize::Information;
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

    #[test]
    fn rejected_and_short_reads_leave_the_port_usable() -> eyre::Result<()> {
        let source = tempfile::NamedTempFile::new()?;
        source.as_file().set_len(512)?;
        let path = source.path().easy_pcwstr()?;
        // SAFETY: the path is valid and the returned file handle is immediately owned.
        let file_handle = unsafe {
            Owned::new(CreateFileW(
                &path,
                FILE_GENERIC_READ.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OVERLAPPED,
                None,
            )?)
        };
        // SAFETY: the file is valid and the returned completion port is immediately owned.
        let completion_port =
            unsafe { Owned::new(CreateIoCompletionPort(*file_handle, None, 0, 0)?) };

        let request =
            PhysicalReadRequest::new(Information::new::<byte>(0), Information::new::<byte>(512));
        let error = ActivePhysicalReadRequest::new(request)
            .send(HANDLE::default())
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("ReadFile failed to queue request")
        );

        ActivePhysicalReadRequest::new(PhysicalReadRequest::new(
            Information::new::<byte>(0),
            Information::new::<byte>(1024),
        ))
        .send(*file_handle)?;
        // SAFETY: this live port contains only our one submitted request's completion.
        let error = unsafe { ActivePhysicalReadRequest::receive(*completion_port) }.unwrap_err();
        assert!(error.to_string().contains("Short physical read"));

        ActivePhysicalReadRequest::new(request).send(*file_handle)?;
        // SAFETY: the prior request was reclaimed; only this request can complete here.
        let (completed_request, data) =
            unsafe { ActivePhysicalReadRequest::receive(*completion_port) }?;
        assert_eq!(completed_request, request);
        assert_eq!(data, [0; 512]);
        Ok(())
    }

    // Invariant check: `overlapped` must be the first field.
    // This ensures `&mut req.overlapped as *mut _` equals
    // `&mut req as *mut _` so that we can cast the lp_overlapped pointer
    // back to *mut ReadRequest on completion safely.
    #[test]
    fn assert_pointer_alignment() {
        let mut dummy = Box::new(ActivePhysicalReadRequest {
            overlapped: OVERLAPPED::default(),
            buffer: Vec::new(),
            original: PhysicalReadRequest::new(
                Information::new::<byte>(0),
                Information::new::<byte>(0),
            ),
        });
        let req_ref = dummy.as_mut();
        let parent_ptr = from_mut(req_ref) as usize;
        let child_ptr = (&raw mut req_ref.overlapped) as usize;
        assert_eq!(
            parent_ptr, child_ptr,
            "ReadRequest.overlapped must be the first field (offset 0)"
        );
        drop(dummy);
    }
}
