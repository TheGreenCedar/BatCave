//! Opt-in local proof recording; absent from ordinary sessions unless the caller
//! supplies an exclusive output path. No telemetry is uploaded or persisted in
//! the user's runtime directory.
use serde::{Deserialize, Serialize};
use std::{
    fs::OpenOptions,
    io::Write,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, SyncSender},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

pub(crate) const WARMUP_SECONDS: u64 = 30;
pub(crate) const MEASUREMENT_SECONDS: u64 = 120;
const MAX_EVENTS: usize = 4096;

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Observation {
    Publication {
        publication_seq: u64,
        age_ms: f64,
        process_count: u32,
        interval_ms: u32,
    },
    Interaction {
        duration_ms: f64,
    },
}

impl Observation {
    fn valid(self) -> bool {
        match self {
            Self::Publication {
                publication_seq,
                age_ms,
                process_count,
                interval_ms,
            } => {
                publication_seq > 0
                    && age_ms.is_finite()
                    && (0.0..=3_600_000.0).contains(&age_ms)
                    && process_count <= 1_000_000
                    && (500..=5_000).contains(&interval_ms)
            }
            Self::Interaction { duration_ms } => {
                duration_ms.is_finite() && (0.0..=60_000.0).contains(&duration_ms)
            }
        }
    }
}

type RecordedObservation = (f64, Observation, bool);
type Admission = Arc<Mutex<Option<SyncSender<RecordedObservation>>>>;

struct Session {
    started: Instant,
    sender: Admission,
    rejected: Arc<AtomicUsize>,
}

#[derive(Default)]
pub(crate) struct DesktopProbe {
    session: Option<Session>,
}

impl DesktopProbe {
    pub(crate) fn from_env(narrative: &crate::narratives::NarrativeState) -> Result<Self, String> {
        let Some(path) = std::env::var_os("BATCAVE_DESKTOP_PROBE_PATH") else {
            return Ok(Self::default());
        };
        let path = std::path::PathBuf::from(path);
        if !path.is_absolute() {
            return Err("desktop_probe_path_must_be_absolute".into());
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|error| format!("desktop_probe_create_failed:{error}"))?;
        let started = Instant::now();
        let started_at_unix_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| "desktop_probe_clock_invalid")?
            .as_secs_f64()
            * 1000.0;
        let header = serde_json::json!({
            "kind": "header", "schema_version": 1, "pid": std::process::id(),
            "release_identity": crate::protocol::release_identity(),
            "started_at_unix_ms": started_at_unix_ms,
            "narrative_preferences_path": narrative.preferences_path(),
            "enhanced_narratives": narrative.preferences().enhanced_narratives,
            "warmup_seconds": WARMUP_SECONDS, "measurement_seconds": MEASUREMENT_SECONDS,
            "interaction_boundary": "trusted_dom_event_to_second_animation_frame",
            "publication_boundary": "runtime_publication_to_renderer_second_animation_frame"
        });
        writeln!(file, "{header}")
            .map_err(|error| format!("desktop_probe_header_failed:{error}"))?;
        let (sender, receiver) = mpsc::sync_channel::<(f64, Observation, bool)>(256);
        let sender = Arc::new(Mutex::new(Some(sender)));
        let worker_sender = sender.clone();
        let rejected = Arc::new(AtomicUsize::new(0));
        let worker_rejected = rejected.clone();
        std::thread::Builder::new()
            .name("batcave-desktop-proof".into())
            .spawn(move || {
                record_window(
                    &mut file,
                    started,
                    Duration::from_secs(WARMUP_SECONDS + MEASUREMENT_SECONDS),
                    receiver,
                    worker_sender,
                    worker_rejected,
                );
                let _ = file.sync_all();
            })
            .map_err(|error| format!("desktop_probe_spawn_failed:{error}"))?;
        Ok(Self {
            session: Some(Session {
                started,
                sender,
                rejected,
            }),
        })
    }

    pub(crate) fn enabled(&self) -> bool {
        self.session.as_ref().is_some_and(|session| {
            session.started.elapsed() < Duration::from_secs(WARMUP_SECONDS + MEASUREMENT_SECONDS)
        })
    }

    pub(crate) fn record(
        &self,
        observation: Observation,
        enhanced_narratives: bool,
    ) -> Result<(), String> {
        if !self.enabled() {
            return Ok(());
        }
        let session = self.session.as_ref().expect("enabled session");
        let admission = session
            .sender
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(sender) = admission.as_ref() else {
            return Ok(());
        };
        if !self.enabled() {
            return Ok(());
        }
        if !observation.valid() {
            session.rejected.fetch_add(1, Ordering::Relaxed);
            return Err("desktop_probe_observation_invalid".into());
        }
        sender
            .try_send((
                session.started.elapsed().as_secs_f64() * 1000.0,
                observation,
                enhanced_narratives,
            ))
            .map_err(|_| {
                session.rejected.fetch_add(1, Ordering::Relaxed);
                "desktop_probe_recorder_busy".to_string()
            })
    }
}

fn record_window(
    writer: &mut impl Write,
    started: Instant,
    duration: Duration,
    receiver: mpsc::Receiver<RecordedObservation>,
    admission: Admission,
    rejected: Arc<AtomicUsize>,
) {
    let deadline = started + duration;
    let mut events = 0;
    let mut write_failed = false;
    {
        let mut record = |(elapsed_ms, observation, enhanced_narratives): RecordedObservation| {
            events += 1;
            if events > MAX_EVENTS {
                rejected.fetch_add(1, Ordering::Relaxed);
                return true;
            }
            let line = serde_json::json!({"kind":"observation", "elapsed_ms":elapsed_ms,
                "observation":observation, "enhanced_narratives":enhanced_narratives});
            writeln!(writer, "{line}").is_ok()
        };
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            match receiver.recv_timeout(remaining.min(Duration::from_secs(1))) {
                Ok(row) => {
                    if !record(row) {
                        write_failed = true;
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        // Close under the same lock as admission, then drain the accepted bounded
        // queue. No sender can race this drain or report success after the footer.
        admission
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        for row in receiver.try_iter() {
            if !record(row) {
                write_failed = true;
            }
        }
    }
    let footer = serde_json::json!({"kind":"footer", "elapsed_ms":started.elapsed().as_secs_f64()*1000.0,
        "rejected_events":rejected.load(Ordering::Relaxed), "write_failed":write_failed});
    let _ = writeln!(writer, "{footer}");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deadline_closes_admission_and_preserves_already_accepted_slow_events() {
        let (sender, receiver) = mpsc::sync_channel(3);
        for duration_ms in [5.0, 25.0, 999.0] {
            sender
                .send((149_999.0, Observation::Interaction { duration_ms }, false))
                .unwrap();
        }
        let admission = Arc::new(Mutex::new(Some(sender)));
        let mut bytes = Vec::new();
        record_window(
            &mut bytes,
            Instant::now(),
            Duration::ZERO,
            receiver,
            admission.clone(),
            Arc::new(AtomicUsize::new(0)),
        );
        assert!(admission.lock().unwrap().is_none());
        let lines = String::from_utf8(bytes)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[2]["observation"]["duration_ms"], 999.0);
        assert_eq!(lines[3]["rejected_events"], 0);
        assert_eq!(lines[3]["write_failed"], false);
    }

    #[test]
    fn proof_observations_reject_nonfinite_impossible_and_unknown_fields() {
        assert!(!Observation::Interaction {
            duration_ms: f64::NAN
        }
        .valid());
        assert!(!Observation::Interaction { duration_ms: -1.0 }.valid());
        assert!(!Observation::Publication {
            publication_seq: 0,
            age_ms: 0.0,
            process_count: 500,
            interval_ms: 1000
        }
        .valid());
        assert!(Observation::Publication {
            publication_seq: 1,
            age_ms: 12.0,
            process_count: 500,
            interval_ms: 1000
        }
        .valid());
        assert!(serde_json::from_value::<Observation>(
            serde_json::json!({"kind":"interaction","duration_ms":1.0,"extra":true})
        )
        .is_err());
    }
}
