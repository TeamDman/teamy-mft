use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

#[derive(Debug)]
pub(crate) struct CtrlCSenderForwarder {
    stop: Arc<AtomicBool>,
    join: std::thread::JoinHandle<eyre::Result<()>>,
}

#[derive(Debug)]
pub(crate) struct CtrlCFlagForwarder {
    stop: Arc<AtomicBool>,
    join: std::thread::JoinHandle<()>,
}

impl CtrlCSenderForwarder {
    pub(crate) fn spawn(cancel_tx: vox::Tx<u8>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_thread = Arc::clone(&stop);
        let join = std::thread::spawn(move || {
            while !stop_for_thread.load(Ordering::Relaxed)
                && !crate::windows_utils::ctrl_c::interrupted()
            {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            if stop_for_thread.load(Ordering::Relaxed) {
                return Ok(());
            }
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            runtime.block_on(async move {
                let _ = cancel_tx.send(1).await;
                let _ = cancel_tx.close(Vec::new()).await;
            });
            Ok(())
        });
        Self { stop, join }
    }

    pub(crate) fn finish(self) -> eyre::Result<()> {
        self.stop.store(true, Ordering::Relaxed);
        self.join.join().map_err(|join_error| {
            eyre::eyre!("Daemon cancel forwarder thread panicked: {join_error:?}")
        })?
    }
}

impl CtrlCFlagForwarder {
    pub(crate) fn spawn(cancel: Arc<AtomicBool>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_thread = Arc::clone(&stop);
        let join = std::thread::spawn(move || {
            while !stop_for_thread.load(Ordering::Relaxed)
                && !crate::windows_utils::ctrl_c::interrupted()
            {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            if stop_for_thread.load(Ordering::Relaxed) {
                return;
            }
            cancel.store(true, Ordering::Relaxed);
        });
        Self { stop, join }
    }

    pub(crate) fn finish(self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.join.join();
    }
}

#[cfg(test)]
mod tests {
    use super::CtrlCFlagForwarder;
    use super::CtrlCSenderForwarder;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::Ordering;

    #[test]
    fn ctrl_c_sender_forwarder_finishes_when_stopped_without_interrupt() {
        let (cancel_tx, _cancel_rx) = vox::channel::<u8>();
        let forwarder = CtrlCSenderForwarder::spawn(cancel_tx);
        forwarder
            .finish()
            .expect("forwarder should stop cleanly without ctrl+c");
    }

    #[test]
    fn ctrl_c_flag_forwarder_finishes_when_stopped_without_interrupt() {
        let cancel = Arc::new(AtomicBool::new(false));
        let forwarder = CtrlCFlagForwarder::spawn(Arc::clone(&cancel));
        forwarder.finish();
        assert!(!cancel.load(Ordering::Relaxed));
    }
}