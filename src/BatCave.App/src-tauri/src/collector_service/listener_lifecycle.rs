use std::{
    sync::atomic::{AtomicBool, Ordering},
    thread::JoinHandle,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ListenerState {
    Connected,
    Abandoned,
    Listening,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ListenerWait {
    Connected,
    Abandoned,
    Stopped,
}

pub(crate) fn wait_for_listener(
    stop: &AtomicBool,
    mut terminal_failure: impl FnMut() -> Result<Option<String>, String>,
    mut connect_state: impl FnMut() -> Result<ListenerState, String>,
    mut wait: impl FnMut(),
) -> Result<ListenerWait, String> {
    loop {
        if stop.load(Ordering::Acquire) {
            return Ok(ListenerWait::Stopped);
        }
        if let Some(failure) = terminal_failure()? {
            return Err(failure);
        }
        match connect_state()? {
            ListenerState::Connected => return Ok(ListenerWait::Connected),
            ListenerState::Abandoned => return Ok(ListenerWait::Abandoned),
            ListenerState::Listening => wait(),
        }
    }
}

pub(crate) fn shutdown_workers(stop: &AtomicBool, workers: &mut Vec<JoinHandle<()>>) {
    stop.store(true, Ordering::Release);
    for worker in workers.drain(..) {
        let _ = worker.join();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::Cell,
        sync::{mpsc, Arc},
    };

    #[test]
    fn fatal_publication_interrupts_an_idle_listener_without_a_client() {
        let stop = AtomicBool::new(false);
        let terminal_polls = Cell::new(0_u32);
        let connect_polls = Cell::new(0_u32);

        let result = wait_for_listener(
            &stop,
            || {
                terminal_polls.set(terminal_polls.get() + 1);
                Ok((terminal_polls.get() == 2)
                    .then(|| "collector_service_snapshot_fatal".to_string()))
            },
            || {
                connect_polls.set(connect_polls.get() + 1);
                Ok(ListenerState::Listening)
            },
            || {},
        );

        assert_eq!(result, Err("collector_service_snapshot_fatal".to_string()));
        assert_eq!(terminal_polls.get(), 2);
        assert_eq!(connect_polls.get(), 1);
    }

    #[test]
    fn listener_reports_connected_and_abandoned_states() {
        let stop = AtomicBool::new(false);
        for (state, expected) in [
            (ListenerState::Connected, ListenerWait::Connected),
            (ListenerState::Abandoned, ListenerWait::Abandoned),
        ] {
            assert_eq!(
                wait_for_listener(&stop, || Ok(None), || Ok(state), || {}),
                Ok(expected)
            );
        }
    }

    #[test]
    fn listener_shutdown_signals_and_joins_active_workers() {
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let (started_tx, started_rx) = mpsc::channel();
        let (finished_tx, finished_rx) = mpsc::channel();
        let mut workers = vec![std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            while !worker_stop.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
            finished_tx.send(()).unwrap();
        })];
        started_rx.recv().unwrap();

        shutdown_workers(stop.as_ref(), &mut workers);

        assert!(stop.load(Ordering::Acquire));
        assert!(workers.is_empty());
        finished_rx.recv().unwrap();
    }
}
