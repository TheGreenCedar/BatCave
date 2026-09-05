//! Bounded current-user writer. Collection never waits for serialization or storage.
use crate::{
    contracts::{RuntimePersistence, RuntimePersistenceDurability, RuntimePersistenceKind},
    persistence::{
        DiagnosticWriteOutcome, PersistenceFailure, PersistenceFailureCode, PersistenceOperation,
        PersistenceWriteEffect, RuntimePersistenceCoordinator, UserStorageComponent,
    },
};
use serde::Serialize;
#[cfg(windows)]
use std::path::Path;
#[cfg(any(windows, test))]
use std::path::PathBuf;
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Condvar, Mutex},
    thread::JoinHandle,
    time::{Duration, Instant},
};

const QUEUE_CAPACITY: usize = 32;
type Operation = Box<dyn FnOnce(&mut RuntimePersistenceCoordinator) + Send>;
struct Job {
    generation: u64,
    at_ms: u64,
    component: Option<UserStorageComponent>,
    coalescible: bool,
    operation: Operation,
}
struct Queue {
    generation: u64,
    rejected: HashMap<UserStorageComponent, (u64, u64, PersistenceFailure)>,
    failed_writes: HashSet<UserStorageComponent>,
    jobs: VecDeque<Job>,
    active: Option<Option<UserStorageComponent>>,
    health: RuntimePersistence,
    closed: bool,
    completed: bool,
}
struct Shared {
    queue: Mutex<Queue>,
    changed: Condvar,
}
struct Writer {
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

pub(crate) struct BackgroundPersistence {
    #[cfg(windows)]
    directory: PathBuf,
    inline: Option<RuntimePersistenceCoordinator>,
    writer: Option<Writer>,
}

impl From<RuntimePersistenceCoordinator> for BackgroundPersistence {
    fn from(value: RuntimePersistenceCoordinator) -> Self {
        Self {
            #[cfg(windows)]
            directory: value.runtime_directory().to_path_buf(),
            inline: Some(value),
            writer: None,
        }
    }
}
impl BackgroundPersistence {
    pub(crate) fn start(&mut self) -> Result<(), String> {
        if self.writer.is_some() {
            return Ok(());
        }
        let mut coordinator = self.inline.take().ok_or("persistence_writer_unavailable")?;
        let shared = Arc::new(Shared {
            queue: Mutex::new(Queue {
                generation: 0,
                rejected: HashMap::new(),
                failed_writes: HashSet::new(),
                jobs: VecDeque::new(),
                active: None,
                health: coordinator.health(),
                closed: false,
                completed: false,
            }),
            changed: Condvar::new(),
        });
        let worker_shared = shared.clone();
        let thread = std::thread::Builder::new()
            .name("batcave-local-writer".into())
            .spawn(move || loop {
                let job = {
                    let mut queue = worker_shared
                        .queue
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    while queue.jobs.is_empty() && !queue.closed {
                        queue = worker_shared
                            .changed
                            .wait(queue)
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                    }
                    let Some(job) = queue.jobs.pop_front() else {
                        queue.completed = true;
                        worker_shared.changed.notify_all();
                        break;
                    };
                    queue.active = Some(job.component);
                    job
                };
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    (job.operation)(&mut coordinator)
                }));
                let mut queue = worker_shared
                    .queue
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                queue.health = coordinator.health();
                queue.active = None;
                if outcome.is_err() {
                    queue.closed = true;
                    queue.completed = true;
                    let failure = queue_failure("writer panicked");
                    for component in [
                        UserStorageComponent::Settings,
                        UserStorageComponent::WarmCache,
                        UserStorageComponent::Diagnostics,
                    ] {
                        let generation = queue.generation;
                        queue
                            .rejected
                            .insert(component, (generation, job.at_ms, failure.clone()));
                    }
                    queue.jobs.clear();
                    worker_shared.changed.notify_all();
                    break;
                }
                if let Some(component) = job.component {
                    if queue.health.components.iter().any(|state| {
                        matches_kind(Some(component), state.kind)
                            && state.state == crate::contracts::RuntimePersistenceState::Healthy
                    }) {
                        queue.failed_writes.remove(&component);
                    } else {
                        queue.failed_writes.insert(component);
                    }
                    if queue
                        .rejected
                        .get(&component)
                        .is_some_and(|(generation, _, _)| *generation < job.generation)
                        && queue.health.components.iter().any(|state| {
                            matches_kind(Some(component), state.kind)
                                && state.state == crate::contracts::RuntimePersistenceState::Healthy
                        })
                    {
                        queue.rejected.remove(&component);
                    }
                }
                worker_shared.changed.notify_all();
            })
            .map_err(|error| format!("persistence_writer_spawn_failed:{error}"))?;
        self.writer = Some(Writer {
            shared,
            thread: Some(thread),
        });
        Ok(())
    }

    pub(crate) fn is_background(&self) -> bool {
        self.writer.is_some()
    }
    #[cfg(windows)]
    pub(crate) fn inline_mut(&mut self) -> Option<&mut RuntimePersistenceCoordinator> {
        self.inline.as_mut()
    }
    #[cfg(windows)]
    pub(crate) fn runtime_directory(&self) -> &Path {
        &self.directory
    }
    pub(crate) fn health(&self) -> RuntimePersistence {
        if let Some(inline) = &self.inline {
            return inline.health();
        }
        let writer = self.writer.as_ref().expect("started writer");
        let queue = writer
            .shared
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut health = queue.health.clone();
        for component in &mut health.components {
            let pending = queue
                .jobs
                .iter()
                .any(|job| matches_kind(job.component, component.kind))
                || queue
                    .active
                    .is_some_and(|active| matches_kind(active, component.kind));
            if pending {
                component.durability = RuntimePersistenceDurability::SessionOnly;
            }
        }
        for (kind, (_, failed_at_ms, failure)) in &queue.rejected {
            if let Some(component) = health
                .components
                .iter_mut()
                .find(|component| matches_kind(Some(*kind), component.kind))
            {
                component.state = crate::contracts::RuntimePersistenceState::Degraded;
                component.durability = RuntimePersistenceDurability::SessionOnly;
                component.active_failure =
                    Some(crate::persistence::runtime_failure(failure, *failed_at_ms));
            }
        }
        if !queue.rejected.is_empty()
            && health.state == crate::contracts::RuntimePersistenceState::Healthy
        {
            health.state = crate::contracts::RuntimePersistenceState::Degraded;
        }
        health
    }

    fn submit(&mut self, mut job: Job) -> Result<(), PersistenceFailure> {
        if let Some(inline) = &mut self.inline {
            (job.operation)(inline);
            return Ok(());
        }
        let Some(writer) = &self.writer else {
            return Err(queue_failure("writer unavailable"));
        };
        let mut queue = writer
            .shared
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        queue.generation = queue.generation.saturating_add(1);
        job.generation = queue.generation;
        if queue.closed {
            let failure = queue_failure("writer closed");
            if let Some(component) = job.component {
                queue
                    .rejected
                    .insert(component, (job.generation, job.at_ms, failure.clone()));
            }
            return Err(failure);
        }
        // Only disposable cache writes coalesce. Moving the replacement to the tail
        // preserves ordering across settings and cache purges.
        if job.coalescible {
            queue.jobs.retain(|pending| !pending.coalescible);
        }
        if queue.jobs.len() >= QUEUE_CAPACITY {
            let failure = queue_failure("writer queue full");
            if let Some(component) = job.component {
                queue
                    .rejected
                    .insert(component, (job.generation, job.at_ms, failure.clone()));
            }
            return Err(failure);
        }
        queue.jobs.push_back(job);
        writer.shared.changed.notify_one();
        Ok(())
    }

    pub(crate) fn write_json<T: Serialize + Clone + Send + 'static>(
        &mut self,
        component: UserStorageComponent,
        value: &T,
        now_ms: u64,
    ) -> Result<(), PersistenceFailure> {
        if let Some(inline) = &mut self.inline {
            return inline.write_json(component, value, now_ms);
        }
        let value = value.clone();
        self.submit(Job { generation: 0, at_ms: now_ms, component: Some(component), coalescible: component == UserStorageComponent::WarmCache, operation: Box::new(move |coordinator| { if coordinator.write_json(component, &value, now_ms).is_ok() && component == UserStorageComponent::Settings {
            coordinator.retry_diagnostics();
            let event = serde_json::json!({"ts_ms": now_ms, "category":"persistence", "payload":{"message":"settings persisted"}});
            let _ = coordinator.record_diagnostic(&event, now_ms);
        } }) })
    }

    pub(crate) fn remove(
        &mut self,
        component: UserStorageComponent,
        now_ms: u64,
    ) -> Result<(), PersistenceFailure> {
        if let Some(inline) = &mut self.inline {
            return inline.remove(component, now_ms);
        }
        self.submit(Job {
            generation: 0,
            at_ms: now_ms,
            component: Some(component),
            coalescible: false,
            operation: Box::new(move |coordinator| {
                let _ = coordinator.remove(component, now_ms);
            }),
        })
    }

    pub(crate) fn record_diagnostic<T: Serialize + Clone + Send + 'static>(
        &mut self,
        value: &T,
        now_ms: u64,
    ) -> DiagnosticWriteOutcome {
        if let Some(inline) = &mut self.inline {
            return inline.record_diagnostic(value, now_ms);
        }
        let value = value.clone();
        match self.submit(Job {
            generation: 0,
            at_ms: now_ms,
            component: Some(UserStorageComponent::Diagnostics),
            coalescible: false,
            operation: Box::new(move |coordinator| {
                let _ = coordinator.record_diagnostic(&value, now_ms);
            }),
        }) {
            Ok(()) => DiagnosticWriteOutcome::Suppressed, // queued, not a durable acknowledgement
            Err(error) => DiagnosticWriteOutcome::Failed(error),
        }
    }

    pub(crate) fn retry_diagnostics(&mut self) {
        let _ = self.submit(Job {
            generation: 0,
            at_ms: 0,
            component: None,
            coalescible: false,
            operation: Box::new(|coordinator| coordinator.retry_diagnostics()),
        });
    }

    #[cfg(windows)]
    pub(crate) fn migration(
        &mut self,
        action: impl FnOnce(&mut RuntimePersistenceCoordinator) -> Result<(), String> + Send + 'static,
    ) -> std::sync::mpsc::Receiver<Result<(), String>> {
        let (sender, receiver) = std::sync::mpsc::channel();
        let failed_sender = sender.clone();
        if let Err(error) = self.submit(Job {
            generation: 0,
            at_ms: 0,
            component: None,
            coalescible: false,
            operation: Box::new(move |coordinator| {
                let _ = sender.send(action(coordinator));
            }),
        }) {
            let _ = failed_sender.send(Err(error.summary));
        }
        receiver
    }

    #[cfg(test)]
    pub(crate) fn block_for_test(&mut self) -> std::sync::mpsc::Sender<()> {
        let (entered, wait_entered) = std::sync::mpsc::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        self.submit(Job {
            generation: 0,
            at_ms: 0,
            component: None,
            coalescible: false,
            operation: Box::new(move |_| {
                let _ = entered.send(());
                let _ = blocked.recv();
            }),
        })
        .unwrap();
        wait_entered.recv_timeout(Duration::from_secs(2)).unwrap();
        release
    }

    pub(crate) fn flush(&self, timeout: Duration) -> Result<(), String> {
        let Some(writer) = &self.writer else {
            return Ok(());
        };
        let deadline = Instant::now() + timeout;
        let mut queue = writer
            .shared
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        while !queue.jobs.is_empty() || queue.active.is_some() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("persistence_flush_timeout".into());
            }
            queue = writer
                .shared
                .changed
                .wait_timeout(queue, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        if queue.closed || !queue.rejected.is_empty() || !queue.failed_writes.is_empty() {
            return Err("persistence_flush_failed".into());
        }
        Ok(())
    }
}

impl Drop for BackgroundPersistence {
    fn drop(&mut self) {
        if let Some(writer) = &mut self.writer {
            let mut queue = writer
                .shared
                .queue
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            queue.closed = true;
            writer.shared.changed.notify_one();
            let completed = queue.completed;
            drop(queue);
            if completed || writer.thread.as_ref().is_some_and(JoinHandle::is_finished) {
                if let Some(thread) = writer.thread.take() {
                    let _ = thread.join();
                }
            }
        }
    }
}
fn matches_kind(component: Option<UserStorageComponent>, kind: RuntimePersistenceKind) -> bool {
    matches!(
        (component, kind),
        (
            Some(UserStorageComponent::Settings),
            RuntimePersistenceKind::Settings
        ) | (
            Some(UserStorageComponent::WarmCache),
            RuntimePersistenceKind::WarmCache
        ) | (
            Some(UserStorageComponent::Diagnostics),
            RuntimePersistenceKind::Diagnostics
        )
    )
}
fn queue_failure(summary: &str) -> PersistenceFailure {
    PersistenceFailure {
        code: PersistenceFailureCode::IoFailure,
        operation: PersistenceOperation::Write,
        path: None,
        retryable: true,
        write_effect: PersistenceWriteEffect::NotCommitted,
        summary: summary.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn writer(label: &str) -> (BackgroundPersistence, PathBuf) {
        let path =
            std::env::temp_dir().join(format!("batcave-writer-{}-{label}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let mut writer = BackgroundPersistence::from(
            RuntimePersistenceCoordinator::for_current_user_directory(path.clone(), 1),
        );
        writer.start().unwrap();
        (writer, path)
    }

    #[test]
    fn rejected_latest_settings_stay_unsaved_after_older_writes_and_recover_only_on_new_write() {
        let (mut writer, path) = writer("overflow");
        let release = writer.block_for_test();
        for seq in 0..QUEUE_CAPACITY {
            writer
                .write_json(
                    UserStorageComponent::Settings,
                    &serde_json::json!({"seq":seq}),
                    seq as u64,
                )
                .unwrap();
        }
        assert!(writer
            .write_json(
                UserStorageComponent::Settings,
                &serde_json::json!({"seq":999}),
                999
            )
            .is_err());
        release.send(()).unwrap();
        assert!(writer.flush(Duration::from_secs(3)).is_err());
        writer
            .write_json(
                UserStorageComponent::WarmCache,
                &serde_json::json!({"seq":1}),
                1000,
            )
            .unwrap();
        assert!(writer.flush(Duration::from_secs(3)).is_err());
        let settings = writer
            .health()
            .components
            .into_iter()
            .find(|c| c.kind == RuntimePersistenceKind::Settings)
            .unwrap();
        assert_eq!(
            settings.state,
            crate::contracts::RuntimePersistenceState::Degraded
        );
        assert_eq!(
            settings.durability,
            RuntimePersistenceDurability::SessionOnly
        );
        writer
            .write_json(
                UserStorageComponent::Settings,
                &serde_json::json!({"seq":1001}),
                1001,
            )
            .unwrap();
        writer.flush(Duration::from_secs(3)).unwrap();
        assert_eq!(
            writer.health().state,
            crate::contracts::RuntimePersistenceState::Healthy
        );
        drop(writer);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn cache_coalescing_respects_purge_barriers() {
        let (mut writer, path) = writer("purge");
        let release = writer.block_for_test();
        writer
            .write_json(
                UserStorageComponent::WarmCache,
                &serde_json::json!({"seq":1}),
                1,
            )
            .unwrap();
        writer.remove(UserStorageComponent::WarmCache, 2).unwrap();
        writer
            .write_json(
                UserStorageComponent::WarmCache,
                &serde_json::json!({"seq":3}),
                3,
            )
            .unwrap();
        release.send(()).unwrap();
        writer.flush(Duration::from_secs(3)).unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path.join("warm-cache.json")).unwrap()).unwrap();
        assert_eq!(value["seq"], 3);
        writer.remove(UserStorageComponent::WarmCache, 4).unwrap();
        writer.flush(Duration::from_secs(3)).unwrap();
        assert!(!path.join("warm-cache.json").exists());
        drop(writer);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn writer_panic_closes_queue_and_reports_failure_without_waiting_for_timeout() {
        let (mut writer, path) = writer("panic");
        writer
            .submit(Job {
                generation: 0,
                at_ms: 0,
                component: None,
                coalescible: false,
                operation: Box::new(|_| panic!("injected writer failure")),
            })
            .unwrap();
        let started = Instant::now();
        assert!(writer.flush(Duration::from_secs(3)).is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(writer
            .write_json(UserStorageComponent::Settings, &0, 1)
            .is_err());
        assert_eq!(
            writer.health().state,
            crate::contracts::RuntimePersistenceState::Degraded
        );
        drop(writer);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn failed_settings_write_never_emits_persisted_diagnostic() {
        let (mut writer, path) = writer("failure");
        std::fs::create_dir(path.join("settings.json")).unwrap();
        writer
            .write_json(
                UserStorageComponent::Settings,
                &serde_json::json!({"seq":1}),
                1,
            )
            .unwrap();
        assert!(writer.flush(Duration::from_secs(3)).is_err());
        for file in std::fs::read_dir(&path)
            .unwrap()
            .flatten()
            .filter(|entry| entry.file_type().unwrap().is_file())
        {
            assert!(!std::fs::read_to_string(file.path())
                .unwrap_or_default()
                .contains("settings persisted"));
        }
        drop(writer);
        let _ = std::fs::remove_dir_all(path);
    }

    #[test]
    fn blocked_storage_coalesces_cache_without_blocking_publication_or_claiming_durability() {
        let path = std::env::temp_dir().join(format!("batcave-writer-{}", std::process::id()));
        let mut writer = BackgroundPersistence::from(
            RuntimePersistenceCoordinator::for_current_user_directory(path.clone(), 1),
        );
        writer.start().unwrap();
        let (entered, wait_entered) = std::sync::mpsc::channel();
        let (release, blocked) = std::sync::mpsc::channel();
        writer
            .submit(Job {
                generation: 0,
                at_ms: 0,
                component: None,
                coalescible: false,
                operation: Box::new(move |_| {
                    entered.send(()).unwrap();
                    blocked.recv().unwrap();
                }),
            })
            .unwrap();
        wait_entered.recv_timeout(Duration::from_secs(1)).unwrap();
        let start = Instant::now();
        for seq in 0..100 {
            writer
                .write_json(
                    UserStorageComponent::WarmCache,
                    &serde_json::json!({"seq":seq}),
                    seq,
                )
                .unwrap();
        }
        writer
            .write_json(
                UserStorageComponent::Settings,
                &serde_json::json!({"value":1}),
                100,
            )
            .unwrap();
        assert!(start.elapsed() < Duration::from_millis(100));
        assert_eq!(
            writer
                .writer
                .as_ref()
                .unwrap()
                .shared
                .queue
                .lock()
                .unwrap()
                .jobs
                .len(),
            2
        );
        assert!(writer
            .health()
            .components
            .iter()
            .filter(|c| c.kind != RuntimePersistenceKind::Diagnostics)
            .all(|c| c.durability == RuntimePersistenceDurability::SessionOnly));
        assert!(writer.flush(Duration::from_millis(1)).is_err());
        release.send(()).unwrap();
        writer.flush(Duration::from_secs(3)).unwrap();
        let cache: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path.join("warm-cache.json")).unwrap()).unwrap();
        assert_eq!(cache["seq"], 99);
        drop(writer);
        let _ = std::fs::remove_dir_all(path);
    }
}
