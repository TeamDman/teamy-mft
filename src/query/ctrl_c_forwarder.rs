use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

pub struct CtrlCForwarder<T> {
    stop: Arc<AtomicBool>,
    join: std::thread::JoinHandle<T>,
    cancel_tx: Option<Arc<Mutex<Option<vox::Tx<u8>>>>>,
}

impl<T> std::fmt::Debug for CtrlCForwarder<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CtrlCForwarder").finish_non_exhaustive()
    }
}

impl<T> CtrlCForwarder<T>
where
    T: Send + 'static,
{
    fn spawn(stopped: T, on_interrupt: impl FnOnce() -> T + Send + 'static) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_thread = Arc::clone(&stop);
        let join = std::thread::spawn(move || {
            while !stop_for_thread.load(Ordering::Relaxed)
                && !crate::windows_utils::ctrl_c::interrupted()
            {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            if stop_for_thread.load(Ordering::Relaxed) {
                return stopped;
            }
            on_interrupt()
        });
        Self {
            stop,
            join,
            cancel_tx: None,
        }
    }

    fn join_stopped(self) -> std::thread::Result<T> {
        self.stop.store(true, Ordering::Relaxed);
        self.join.join()
    }
}

impl CtrlCForwarder<eyre::Result<()>> {
    pub fn spawn_sender(cancel_tx: vox::Tx<u8>) -> Self {
        let cancel_tx = Arc::new(Mutex::new(Some(cancel_tx)));
        let mut forwarder = Self::spawn(Ok(()), {
            let cancel_tx = Arc::clone(&cancel_tx);
            move || Self::send_cancel(cancel_tx)
        });
        forwarder.cancel_tx = Some(cancel_tx);
        forwarder
    }

    pub fn request_cancel(&self) -> eyre::Result<()> {
        let Some(cancel_tx) = &self.cancel_tx else {
            return Ok(());
        };
        Self::send_cancel(Arc::clone(cancel_tx))
    }

    pub fn finish(self) -> eyre::Result<()> {
        self.join_stopped().map_err(|join_error| {
            eyre::eyre!("Daemon cancel forwarder thread panicked: {join_error:?}")
        })?
    }

    fn send_cancel(cancel_tx: Arc<Mutex<Option<vox::Tx<u8>>>>) -> eyre::Result<()> {
        let Some(cancel_tx) = cancel_tx
            .lock()
            .map_err(|_| eyre::eyre!("Daemon cancel sender mutex poisoned"))?
            .take()
        else {
            return Ok(());
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()?;
        runtime.block_on(async move {
            let _ = cancel_tx.send(1).await;
            let _ = cancel_tx.close(Vec::new()).await;
        });
        Ok(())
    }
}

impl CtrlCForwarder<()> {
    pub fn spawn_flag(cancel: Arc<AtomicBool>) -> Self {
        Self::spawn((), move || {
            cancel.store(true, Ordering::Relaxed);
        })
    }

    pub fn finish(self) {
        let _ = self.join_stopped();
    }
}

#[cfg(test)]
mod tests {
    use super::CtrlCForwarder;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::Ordering;

    #[test]
    fn ctrl_c_sender_forwarder_finishes_when_stopped_without_interrupt() {
        let (cancel_tx, _cancel_rx) = vox::channel::<u8>();
        let forwarder = CtrlCForwarder::spawn_sender(cancel_tx);
        forwarder
            .finish()
            .expect("forwarder should stop cleanly without ctrl+c");
    }

    #[test]
    fn ctrl_c_flag_forwarder_finishes_when_stopped_without_interrupt() {
        let cancel = Arc::new(AtomicBool::new(false));
        let forwarder = CtrlCForwarder::spawn_flag(Arc::clone(&cancel));
        forwarder.finish();
        assert!(!cancel.load(Ordering::Relaxed));
    }
}
