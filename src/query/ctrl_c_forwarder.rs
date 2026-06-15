use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

#[derive(Debug)]
pub struct CtrlCForwarder<T> {
    stop: Arc<AtomicBool>,
    join: std::thread::JoinHandle<T>,
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
        Self { stop, join }
    }

    fn join_stopped(self) -> std::thread::Result<T> {
        self.stop.store(true, Ordering::Relaxed);
        self.join.join()
    }
}

impl CtrlCForwarder<eyre::Result<()>> {
    pub fn spawn_sender(cancel_tx: vox::Tx<u8>) -> Self {
        Self::spawn(Ok(()), move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            runtime.block_on(async move {
                let _ = cancel_tx.send(1).await;
                let _ = cancel_tx.close(Vec::new()).await;
            });
            Ok(())
        })
    }

    pub fn finish(self) -> eyre::Result<()> {
        self.join_stopped().map_err(|join_error| {
            eyre::eyre!("Daemon cancel forwarder thread panicked: {join_error:?}")
        })?
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