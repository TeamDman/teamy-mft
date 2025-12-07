use crate::read::physical_read_request::PhysicalReadRequest;
use crate::read::physical_read_results::PhysicalReadResultEntry;
use std::any::type_name;
use std::ptr::null_mut;
use tracing::debug;
use tracing::warn;
use uom::si::information::byte;
use windows::Win32::Foundation::ERROR_IO_PENDING;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::Storage::FileSystem::ReadFile;
use windows::Win32::System::IO::GetQueuedCompletionStatus;
use windows::Win32::System::IO::OVERLAPPED;

// NOTE on layout and safety for IOCP:
// We intentionally embed OVERLAPPED as the FIRST field and mark the
// struct as repr(C). With repr(C), field order and the offset of the
// first field are guaranteed, so the address of a ReadRequest value is
// equal to the address of its `overlapped` field. This allows us to pass
// `&mut read_req.overlapped` to ReadFile and later, when IO completes,
// recover the original allocation from the `lpOverlapped` pointer returned
// by GetQueuedCompletionStatus via a cast back to *mut ReadRequest.
//
// Safety invariants relied upon by this file:
// - `overlapped` MUST remain the first field of ReadRequest.
// - ReadRequest MUST remain `#[repr(C)]`.
// - Each queued I/O leaks its Box<ReadRequest> (Box::into_raw) so the
//   allocation outlives the async operation; ownership is reclaimed once
//   the completion is dequeued by converting the raw pointer back with
//   Box::from_raw exactly once.
// - We never move/relocate the allocation after queueing the I/O.
#[repr(C)]
pub struct ActivePhysicalReadRequest {
    pub overlapped: OVERLAPPED,
    pub buffer: Vec<u8>,
    pub response_index: usize,
    pub original: PhysicalReadRequest,
}
impl std::fmt::Debug for ActivePhysicalReadRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct(type_name::<ActivePhysicalReadRequest>())
            .field("file_offset", &self.original.offset)
            .field("length", &self.original.length)
            .field("response_index", &self.response_index)
            .finish()
    }
}

impl ActivePhysicalReadRequest {
    pub fn new(request: PhysicalReadRequest, response_index: usize) -> Self {
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
            overlapped.Anonymous.Anonymous.Offset = (offset & 0xFFFF_FFFF) as u32;
            overlapped.Anonymous.Anonymous.OffsetHigh = ((offset >> 32) & 0xFFFF_FFFF) as u32;
            overlapped
        };
        Self {
            overlapped,
            buffer: vec![0; request.length.get::<byte>()],
            response_index,
            original: request,
        }
    }

    pub fn send(self, file_handle: HANDLE) -> eyre::Result<()> {
        let mut boxed = Box::new(self);

        // We pass a pointer to the embedded OVERLAPPED. Because
        // `overlapped` is the first field of a repr(C) struct, this
        // pointer is also the address of the parent ReadRequest.
        // On completion, IOCP will give us back this same pointer
        // so we can recover the Box<ReadRequest>.
        let overlapped_ptr: *mut OVERLAPPED = &raw mut boxed.overlapped;
        match unsafe {
            ReadFile(
                file_handle,
                Some(&mut *boxed.buffer ),
                None,
                Some(overlapped_ptr),
            )
        } {
            Ok(()) => {}
            Err(e) => {
                if e.code() != ERROR_IO_PENDING.into() {
                    return Err(eyre::eyre!(
                        "ReadFile failed to queue request {boxed:?}: {e:?}"
                    ));
                }
            }
        }

        // Leak the Box so the allocation outlives the async I/O.
        // We reconstitute it with Box::from_raw when the completion
        // is dequeued, ensuring exactly-once free and no leaks.
        let _ = Box::into_raw(boxed);
        Ok(())
    }

    pub fn receive(completion_port: HANDLE) -> eyre::Result<(PhysicalReadResultEntry, usize)> {
        let mut bytes_transferred: u32 = 0;
        let mut completion_key: usize = 0;
        let mut lp_overlapped: *mut OVERLAPPED = null_mut();
        debug!("Waiting for IOCP read completion");
        let res = unsafe {
            GetQueuedCompletionStatus(
                completion_port,
                &raw mut bytes_transferred,
                &raw mut completion_key,
                &raw mut lp_overlapped,
                u32::MAX,
            )
        };
        match res {
            Ok(()) => {
                if lp_overlapped.is_null() {
                    return Err(eyre::eyre!(
                        "IOCP returned success but OVERLAPPED ptr was null"
                    ));
                }
                // Recover original allocation using container_of pattern: lp_overlapped points to the first
                // field (overlapped) so casting back to the parent type is sound under our invariants.
                let req_ptr = lp_overlapped.cast::<ActivePhysicalReadRequest>();
                let boxed_req = unsafe { Box::from_raw(req_ptr) };
                debug!(?boxed_req, bytes_transferred, "Completed IOCP read",);
                let mut data = boxed_req.buffer;
                let copy_len =
                    (bytes_transferred as usize).min(boxed_req.original.length.get::<byte>());
                if copy_len < data.len() {
                    data.truncate(copy_len);
                }
                Ok((
                    PhysicalReadResultEntry {
                        request: boxed_req.original,
                        data,
                    },
                    boxed_req.response_index,
                ))
            }
            Err(e) => {
                if lp_overlapped.is_null() {
                    Err(eyre::eyre!("GetQueuedCompletionStatus failed: {e:?}"))
                } else {
                    // Same recovery path on error: take ownership back
                    // and allow the allocation to be freed when dropped.
                    let req_ptr = lp_overlapped.cast::<ActivePhysicalReadRequest>();
                    let boxed_req = unsafe { Box::from_raw(req_ptr) };
                    Err(eyre::eyre!("I/O operation failed for {boxed_req:?}: {e:?}"))
                }
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use uom::si::usize::Information;

    // Invariant check: `overlapped` must be the first field.
    // This ensures `&mut req.overlapped as *mut _` equals
    // `&mut req as *mut _` so that we can cast the lp_overlapped pointer
    // back to *mut ReadRequest on completion safely.
    #[test]
    fn assert_pointer_alignment() -> eyre::Result<()> {
        let mut dummy = Box::new(ActivePhysicalReadRequest {
            overlapped: OVERLAPPED::default(),
            buffer: Vec::new(),
            response_index: 0,
            original: PhysicalReadRequest::new(
                Information::new::<byte>(0),
                Information::new::<byte>(0),
            ),
        });
        let parent_ptr = (&mut *dummy as *mut ActivePhysicalReadRequest) as usize;
        let child_ptr = (&mut dummy.overlapped as *mut OVERLAPPED) as usize;
        assert_eq!(
            parent_ptr, child_ptr,
            "ReadRequest.overlapped must be the first field (offset 0)"
        );
        drop(dummy);
        Ok(())
    }
}
