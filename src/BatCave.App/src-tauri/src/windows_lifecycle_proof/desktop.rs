use crate::windows_lifecycle_proof_contract::{
    validate_desktop_visible, DesktopCollectorState, DesktopPhase, DesktopPrivilegedSource,
    DesktopProcessObservation, DesktopVisibleObservation,
};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};
use uiautomation::errors::ERR_NOTFOUND;
use uiautomation::patterns::{UIExpandCollapsePattern, UIInvokePattern, UIWindowPattern};
use uiautomation::types::{ControlType, ExpandCollapseState, WindowInteractionState};
use uiautomation::{UIAutomation, UIElement};

const UI_TIMEOUT_MS: u64 = 30_000;
const VISIBLE_POLL_INTERVAL: Duration = Duration::from_millis(100);
const UI_DEPTH: u32 = 32;
const WINDOW_TITLE: &str = "BatCave Monitor";
const DIAGNOSTIC_PREFIXES: [&str; 11] = [
    "Current process",
    "Privileged source",
    "Standard fallback",
    "Protected sample",
    "Fallback process ETW",
    "Collector service",
    "Service version",
    "Service protocol",
    "Minimum desktop",
    "Service release",
    "Service instance",
];

pub(super) struct DesktopWindow {
    automation: UIAutomation,
    window: UIElement,
    process_id: u32,
    started_at_100ns: u64,
    native_window_handle: isize,
}

impl DesktopWindow {
    pub(super) fn open(process: &DesktopProcessObservation) -> Result<Self, String> {
        let process_id = process.process_id;
        let automation =
            UIAutomation::new().map_err(|_| "lifecycle_desktop_uia_initialize_failed")?;
        let window = automation
            .create_matcher()
            .control_type(ControlType::Window)
            .name(WINDOW_TITLE)
            .filter_fn(Box::new(move |element: &UIElement| {
                Ok(element.get_process_id()? == process_id)
            }))
            .depth(3)
            .timeout(UI_TIMEOUT_MS)
            .find_first()
            .map_err(|_| "lifecycle_desktop_window_not_found")?;
        if !window
            .is_enabled()
            .map_err(|_| "lifecycle_desktop_window_enabled_failed")?
            || window
                .is_offscreen()
                .map_err(|_| "lifecycle_desktop_window_visibility_failed")?
        {
            return Err("lifecycle_desktop_window_not_interactive".to_string());
        }
        let pattern = window
            .get_pattern::<UIWindowPattern>()
            .map_err(|_| "lifecycle_desktop_window_pattern_missing")?;
        if pattern
            .is_modal()
            .map_err(|_| "lifecycle_desktop_window_modal_state_failed")?
            || !matches!(
                pattern
                    .get_window_interaction_state()
                    .map_err(|_| "lifecycle_desktop_window_interaction_state_failed")?,
                WindowInteractionState::Running | WindowInteractionState::ReadyForUserInteraction
            )
        {
            return Err("lifecycle_desktop_window_state_invalid".to_string());
        }
        let native_window_handle: isize = window
            .get_native_window_handle()
            .map_err(|_| "lifecycle_desktop_window_handle_missing")?
            .into();
        if native_window_handle == 0 {
            return Err("lifecycle_desktop_window_handle_invalid".to_string());
        }
        super::native::validate_window_process_identity(
            native_window_handle,
            process.process_id,
            process.started_at_100ns,
        )?;
        Ok(Self {
            automation,
            window,
            process_id,
            started_at_100ns: process.started_at_100ns,
            native_window_handle,
        })
    }

    pub(super) fn read_visible(
        &self,
        phase: DesktopPhase,
        allowed_process_ids: &BTreeSet<u32>,
    ) -> Result<DesktopVisibleObservation, String> {
        let dialog = self.ensure_diagnostics_open(allowed_process_ids)?;
        let deadline = Instant::now() + Duration::from_millis(UI_TIMEOUT_MS);
        poll_visible_contract(
            phase,
            || {
                self.verify_window_identity()?;
                self.diagnostic_groups(&dialog, allowed_process_ids)
            },
            || Instant::now() >= deadline,
            || {
                if let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
                    std::thread::sleep(remaining.min(VISIBLE_POLL_INTERVAL));
                }
            },
        )
    }

    fn diagnostic_groups(
        &self,
        dialog: &UIElement,
        allowed_process_ids: &BTreeSet<u32>,
    ) -> Result<Vec<String>, String> {
        let allowed_process_ids = allowed_process_ids.clone();
        let native_window_handle = self.native_window_handle;
        let groups = match self
            .automation
            .create_matcher()
            .from_ref(dialog)
            .control_type(ControlType::Group)
            .filter_fn(Box::new(move |element: &UIElement| {
                let name = element.get_name()?;
                Ok(DIAGNOSTIC_PREFIXES
                    .iter()
                    .any(|prefix| name.starts_with(&format!("{prefix}: ")))
                    || name.starts_with("Service detail: "))
            }))
            .depth(UI_DEPTH)
            .timeout(0)
            .find_all()
        {
            Ok(groups) => groups,
            Err(error) if error.code() == ERR_NOTFOUND => Vec::new(),
            Err(_) => return Err("lifecycle_desktop_diagnostics_rows_failed".to_string()),
        };
        groups
            .into_iter()
            .filter_map(|element| {
                match element_authorized_for_window(
                    &element,
                    &allowed_process_ids,
                    native_window_handle,
                ) {
                    Ok(true) => {}
                    Ok(false) => {
                        return Some(Err(
                            "lifecycle_desktop_diagnostics_row_authority_invalid".to_string()
                        ))
                    }
                    Err(_) => {
                        return Some(Err(
                            "lifecycle_desktop_diagnostics_row_authority_failed".to_string()
                        ))
                    }
                }
                match (element.is_enabled(), element.is_offscreen()) {
                    (Ok(true), Ok(false)) => {
                        Some(element.get_name().map_err(|_| {
                            "lifecycle_desktop_diagnostics_row_name_failed".to_string()
                        }))
                    }
                    (Ok(_), Ok(_)) => None,
                    _ => Some(Err(
                        "lifecycle_desktop_diagnostics_row_state_failed".to_string()
                    )),
                }
            })
            .collect()
    }

    pub(super) fn wait_for_primary_focus(
        &self,
        primary: &DesktopProcessObservation,
    ) -> Result<(), String> {
        self.verify_window_identity()?;
        if primary.process_id != self.process_id
            || primary.started_at_100ns != self.started_at_100ns
        {
            return Err("lifecycle_desktop_primary_window_identity_changed".to_string());
        }
        super::native::wait_for_foreground_window_identity(
            self.native_window_handle,
            primary.process_id,
            primary.started_at_100ns,
        )
    }

    fn verify_window_identity(&self) -> Result<(), String> {
        if self.window.get_process_id().ok() != Some(self.process_id)
            || self.window.get_name().ok().as_deref() != Some(WINDOW_TITLE)
            || !self.window.is_enabled().unwrap_or(false)
            || self.window.is_offscreen().unwrap_or(true)
            || self
                .window
                .get_native_window_handle()
                .ok()
                .map(Into::<isize>::into)
                != Some(self.native_window_handle)
        {
            return Err("lifecycle_desktop_primary_window_identity_changed".to_string());
        }
        super::native::validate_window_process_identity(
            self.native_window_handle,
            self.process_id,
            self.started_at_100ns,
        )
    }

    pub(super) fn close(self) -> Result<(), String> {
        self.window
            .get_pattern::<UIWindowPattern>()
            .and_then(|pattern| pattern.close())
            .map_err(|_| "lifecycle_desktop_window_close_failed".to_string())
    }

    fn ensure_diagnostics_open(
        &self,
        allowed_process_ids: &BTreeSet<u32>,
    ) -> Result<UIElement, String> {
        if let Ok(dialog) = self.find_diagnostics_dialog(allowed_process_ids, 0) {
            self.expand_technical_details(&dialog, allowed_process_ids)?;
            return Ok(dialog);
        }
        let allowed = allowed_process_ids.clone();
        let native_window_handle = self.native_window_handle;
        let buttons = self
            .automation
            .create_matcher()
            .from_ref(&self.window)
            .control_type(ControlType::Button)
            .filter_fn(Box::new(move |element: &UIElement| {
                Ok(is_header_diagnostics_button_name(&element.get_name()?))
            }))
            .depth(UI_DEPTH)
            .timeout(UI_TIMEOUT_MS)
            .find_all()
            .map_err(|_| "lifecycle_desktop_diagnostics_button_missing")?;
        let buttons = buttons
            .into_iter()
            .filter_map(|element| {
                match element_authorized_for_window(&element, &allowed, native_window_handle) {
                    Ok(true) => {}
                    Ok(false) => {
                        return Some(Err(
                            "lifecycle_desktop_diagnostics_button_authority_invalid".to_string(),
                        ))
                    }
                    Err(_) => {
                        return Some(Err(
                            "lifecycle_desktop_diagnostics_button_authority_failed".to_string()
                        ))
                    }
                }
                match (element.is_enabled(), element.is_offscreen()) {
                    (Ok(true), Ok(false)) => Some(Ok(element)),
                    (Ok(_), Ok(_)) => None,
                    _ => Some(Err(
                        "lifecycle_desktop_diagnostics_button_state_failed".to_string()
                    )),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        if buttons.len() != 1 {
            return Err("lifecycle_desktop_diagnostics_button_ambiguous".to_string());
        }
        buttons[0]
            .get_pattern::<UIInvokePattern>()
            .and_then(|pattern| pattern.invoke())
            .map_err(|_| "lifecycle_desktop_diagnostics_open_failed")?;
        let dialog = self.find_diagnostics_dialog(allowed_process_ids, UI_TIMEOUT_MS)?;
        self.expand_technical_details(&dialog, allowed_process_ids)?;
        Ok(dialog)
    }

    fn find_diagnostics_dialog(
        &self,
        allowed_process_ids: &BTreeSet<u32>,
        timeout: u64,
    ) -> Result<UIElement, String> {
        let allowed = allowed_process_ids.clone();
        let native_window_handle = self.native_window_handle;
        let dialogs = self
            .automation
            .create_matcher()
            .from_ref(&self.window)
            .name("Diagnostics")
            .filter_fn(Box::new(move |element: &UIElement| element.is_dialog()))
            .depth(UI_DEPTH)
            .timeout(timeout)
            .find_all()
            .map_err(|_| "lifecycle_desktop_diagnostics_dialog_missing")?;
        let dialogs = dialogs
            .into_iter()
            .filter_map(|element| {
                match element_authorized_for_window(&element, &allowed, native_window_handle) {
                    Ok(true) => {}
                    Ok(false) => {
                        return Some(Err(
                            "lifecycle_desktop_diagnostics_dialog_authority_invalid".to_string(),
                        ))
                    }
                    Err(_) => {
                        return Some(Err(
                            "lifecycle_desktop_diagnostics_dialog_authority_failed".to_string()
                        ))
                    }
                }
                match (element.is_enabled(), element.is_offscreen()) {
                    (Ok(true), Ok(false)) => Some(Ok(element)),
                    (Ok(_), Ok(_)) => None,
                    _ => Some(Err(
                        "lifecycle_desktop_diagnostics_dialog_state_failed".to_string()
                    )),
                }
            })
            .collect::<Result<Vec<_>, _>>()?;
        if dialogs.len() != 1 {
            return Err("lifecycle_desktop_diagnostics_dialog_ambiguous".to_string());
        }
        Ok(dialogs[0].clone())
    }

    fn expand_technical_details(
        &self,
        dialog: &UIElement,
        allowed_process_ids: &BTreeSet<u32>,
    ) -> Result<(), String> {
        let allowed = allowed_process_ids.clone();
        let native_window_handle = self.native_window_handle;
        let technical = self
            .automation
            .create_matcher()
            .from_ref(dialog)
            .name("Technical details")
            .filter_fn(Box::new(move |element: &UIElement| {
                element_owned_by_window(element, &allowed, native_window_handle)
            }))
            .depth(UI_DEPTH)
            .timeout(UI_TIMEOUT_MS)
            .find_first()
            .map_err(|_| "lifecycle_desktop_technical_details_missing")?;
        if let Ok(pattern) = technical.get_pattern::<UIExpandCollapsePattern>() {
            if pattern
                .get_state()
                .map_err(|_| "lifecycle_desktop_technical_details_state_failed")?
                != ExpandCollapseState::Expanded
            {
                pattern
                    .expand()
                    .map_err(|_| "lifecycle_desktop_technical_details_expand_failed")?;
            }
        } else {
            technical
                .get_pattern::<UIInvokePattern>()
                .and_then(|pattern| pattern.invoke())
                .map_err(|_| "lifecycle_desktop_technical_details_expand_failed")?;
        }
        Ok(())
    }
}

fn element_authorized_for_window(
    element: &UIElement,
    allowed_process_ids: &BTreeSet<u32>,
    native_window_handle: isize,
) -> uiautomation::Result<bool> {
    if !allowed_process_ids.contains(&element.get_process_id()?) {
        return Ok(false);
    }
    let element_window: isize = element.get_native_window_handle()?.into();
    Ok(element_window == 0 || element_window == native_window_handle)
}

fn element_owned_by_window(
    element: &UIElement,
    allowed_process_ids: &BTreeSet<u32>,
    native_window_handle: isize,
) -> uiautomation::Result<bool> {
    if !element.is_enabled()? || element.is_offscreen()? {
        return Ok(false);
    }
    if !allowed_process_ids.contains(&element.get_process_id()?) {
        return Ok(false);
    }
    let element_window: isize = element.get_native_window_handle()?.into();
    Ok(element_window == 0 || element_window == native_window_handle)
}

fn is_header_diagnostics_button_name(name: &str) -> bool {
    name.strip_suffix(". Open diagnostics.")
        .is_some_and(|status| !status.trim().is_empty())
}

#[derive(Debug, Eq, PartialEq)]
enum VisibleRead {
    Ready(DesktopVisibleObservation),
    Retryable(String),
    Authority(String),
}

fn poll_visible_contract<Read, Expired, Wait>(
    phase: DesktopPhase,
    mut read: Read,
    mut expired: Expired,
    mut wait: Wait,
) -> Result<DesktopVisibleObservation, String>
where
    Read: FnMut() -> Result<Vec<String>, String>,
    Expired: FnMut() -> bool,
    Wait: FnMut(),
{
    loop {
        match classify_visible_labels(phase, read()?) {
            VisibleRead::Ready(visible) => return Ok(visible),
            VisibleRead::Authority(reason) => return Err(reason),
            VisibleRead::Retryable(reason) => {
                if expired() {
                    return Err(format!("lifecycle_desktop_visible_state_timeout:{reason}"));
                }
                wait();
            }
        }
    }
}

fn classify_visible_labels(
    phase: DesktopPhase,
    labels: impl IntoIterator<Item = String>,
) -> VisibleRead {
    match parse_visible_labels(labels) {
        Ok(visible) => match validate_desktop_visible(phase, &visible) {
            Ok(()) => VisibleRead::Ready(visible),
            Err(reason) => classify_visible_contract_failure(reason),
        },
        Err(failure) => failure.into(),
    }
}

fn classify_visible_contract_failure(reason: String) -> VisibleRead {
    match reason.as_str() {
        "lifecycle_desktop_visible_state_invalid" | "lifecycle_desktop_visible_source_invalid" => {
            VisibleRead::Retryable(reason)
        }
        "lifecycle_desktop_active_identity_invalid"
        | "lifecycle_desktop_incompatible_identity_invalid"
        | "lifecycle_desktop_fallback_identity_invalid" => VisibleRead::Authority(reason),
        _ => VisibleRead::Authority(reason),
    }
}

#[derive(Debug, Eq, PartialEq)]
enum VisibleParseFailure {
    Retryable(String),
    Authority(String),
}

impl From<VisibleParseFailure> for VisibleRead {
    fn from(failure: VisibleParseFailure) -> Self {
        match failure {
            VisibleParseFailure::Retryable(reason) => Self::Retryable(reason),
            VisibleParseFailure::Authority(reason) => Self::Authority(reason),
        }
    }
}

fn parse_visible_labels(
    labels: impl IntoIterator<Item = String>,
) -> Result<DesktopVisibleObservation, VisibleParseFailure> {
    let mut fields = BTreeMap::new();
    for label in labels {
        let (key, value) = label.split_once(": ").ok_or_else(|| {
            VisibleParseFailure::Authority("lifecycle_desktop_diagnostics_row_invalid".to_string())
        })?;
        if fields.insert(key.to_string(), value.to_string()).is_some() {
            return Err(VisibleParseFailure::Authority(
                "lifecycle_desktop_diagnostics_row_duplicate".to_string(),
            ));
        }
    }

    let current_process_standard = required(&fields, "Current process")? == "Standard token";
    if !current_process_standard {
        return Err(VisibleParseFailure::Authority(
            "lifecycle_desktop_visible_process_not_standard".to_string(),
        ));
    }
    let privileged_source = match required(&fields, "Privileged source")? {
        "Installed collector service" => DesktopPrivilegedSource::InstalledCollectorService,
        "None" => DesktopPrivilegedSource::None,
        _ => {
            return Err(VisibleParseFailure::Authority(
                "lifecycle_desktop_visible_privileged_source_invalid".to_string(),
            ))
        }
    };
    let collector_state = match required(&fields, "Collector service")? {
        "Collector service active" => DesktopCollectorState::Active,
        "Collector service not installed" => DesktopCollectorState::NotInstalled,
        "Collector service stopped" => DesktopCollectorState::Stopped,
        "Collector service incompatible" => DesktopCollectorState::Incompatible,
        "Collector service connecting" | "Collector service recovering" => {
            return Err(VisibleParseFailure::Retryable(
                "lifecycle_desktop_visible_collector_state_pending".to_string(),
            ))
        }
        "Collector service unauthorized" => {
            return Err(VisibleParseFailure::Authority(
                "lifecycle_desktop_visible_collector_unauthorized".to_string(),
            ))
        }
        "Collector service failed" => {
            return Err(VisibleParseFailure::Authority(
                "lifecycle_desktop_visible_collector_failed".to_string(),
            ))
        }
        _ => {
            return Err(VisibleParseFailure::Authority(
                "lifecycle_desktop_visible_collector_state_invalid".to_string(),
            ))
        }
    };
    let standard_monitoring_current =
        current_value(required(&fields, "Standard fallback")?, "standard_fallback")?;
    let protected_sample_current =
        current_value(required(&fields, "Protected sample")?, "protected_sample")?;
    let fallback_etw_disabled = match required(&fields, "Fallback process ETW")? {
        "Disabled" => true,
        "Not active" => false,
        "Not verified" => {
            return Err(VisibleParseFailure::Retryable(
                "lifecycle_desktop_fallback_etw_truth_unavailable".to_string(),
            ))
        }
        _ => {
            return Err(VisibleParseFailure::Authority(
                "lifecycle_desktop_fallback_etw_state_invalid".to_string(),
            ))
        }
    };

    Ok(DesktopVisibleObservation {
        current_process_standard,
        collector_state,
        privileged_source,
        standard_monitoring_current,
        protected_sample_current,
        fallback_etw_disabled,
        service_version: optional_value(required(&fields, "Service version")?, "Not reported"),
        service_release_version: optional_value(
            required(&fields, "Service release")?,
            "Not reported",
        ),
        negotiated_protocol_version: optional_protocol(required(&fields, "Service protocol")?)?,
        minimum_desktop_version: optional_value(
            required(&fields, "Minimum desktop")?,
            "Not reported",
        ),
        service_instance_id: optional_value(
            required(&fields, "Service instance")?,
            "Not connected",
        ),
        service_detail: optional_value(required(&fields, "Service detail")?, "None"),
    })
}

fn required<'a>(
    fields: &'a BTreeMap<String, String>,
    key: &str,
) -> Result<&'a str, VisibleParseFailure> {
    fields.get(key).map(String::as_str).ok_or_else(|| {
        VisibleParseFailure::Retryable(format!("lifecycle_desktop_diagnostics_{key}_missing"))
    })
}

fn current_value(value: &str, label: &str) -> Result<bool, VisibleParseFailure> {
    match value {
        "Current" => Ok(true),
        "Not active" | "Unavailable" | "Paused" | "Stale" | "No sample" => Ok(false),
        _ => Err(VisibleParseFailure::Authority(format!(
            "lifecycle_desktop_{label}_state_invalid"
        ))),
    }
}

fn optional_value(value: &str, absent: &str) -> Option<String> {
    (value != absent).then(|| value.to_string())
}

fn optional_protocol(value: &str) -> Result<Option<u16>, VisibleParseFailure> {
    if value == "Not negotiated" {
        return Ok(None);
    }
    value.parse::<u16>().map(Some).map_err(|_| {
        VisibleParseFailure::Authority("lifecycle_desktop_service_protocol_invalid".to_string())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::collections::VecDeque;

    fn active_labels() -> Vec<String> {
        [
            "Current process: Standard token",
            "Privileged source: Installed collector service",
            "Standard fallback: Not active",
            "Protected sample: Current",
            "Fallback process ETW: Not active",
            "Collector service: Collector service active",
            "Service version: 0.2.0-rc.2",
            "Service protocol: 1",
            "Minimum desktop: 0.2.0-rc.2",
            "Service release: 0.2.0-rc.2",
            "Service instance: 00000065-00000000000000000000000000000001",
            "Service detail: None",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    }

    fn connecting_labels() -> Vec<String> {
        [
            "Current process: Standard token",
            "Privileged source: None",
            "Standard fallback: No sample",
            "Protected sample: No sample",
            "Fallback process ETW: Disabled",
            "Collector service: Collector service connecting",
            "Service version: Not reported",
            "Service protocol: Not negotiated",
            "Minimum desktop: Not reported",
            "Service release: Not reported",
            "Service instance: Not connected",
            "Service detail: collector_service_snapshot_not_ready",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    }

    fn missing_service_labels() -> Vec<String> {
        [
            "Current process: Standard token",
            "Privileged source: None",
            "Standard fallback: Current",
            "Protected sample: Unavailable",
            "Fallback process ETW: Disabled",
            "Collector service: Collector service not installed",
            "Service version: Not reported",
            "Service protocol: Not negotiated",
            "Minimum desktop: Not reported",
            "Service release: Not reported",
            "Service instance: Not connected",
            "Service detail: collector_service_open_failed:1060",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    }

    fn incompatible_service_labels() -> Vec<String> {
        [
            "Current process: Standard token",
            "Privileged source: None",
            "Standard fallback: Current",
            "Protected sample: Unavailable",
            "Fallback process ETW: Disabled",
            "Collector service: Collector service incompatible",
            "Service version: 0.2.0-rc.3",
            "Service protocol: Not negotiated",
            "Minimum desktop: Not reported",
            "Service release: 0.2.0-rc.3",
            "Service instance: Not connected",
            "Service detail: collector_service_desktop_release_incompatible",
        ]
        .into_iter()
        .map(str::to_string)
        .collect()
    }

    fn replace_label(labels: Vec<String>, key: &str, value: &str) -> Vec<String> {
        let prefix = format!("{key}: ");
        labels
            .into_iter()
            .map(|label| {
                if label.starts_with(&prefix) {
                    format!("{prefix}{value}")
                } else {
                    label
                }
            })
            .collect()
    }

    fn assert_visible_poll_stops_without_wait(
        phase: DesktopPhase,
        labels: Vec<String>,
        expected_reason: &str,
    ) {
        let waits = Cell::new(0);
        let result = poll_visible_contract(
            phase,
            || Ok(labels.clone()),
            || panic!("authority failure must not consult the retry deadline"),
            || waits.set(waits.get() + 1),
        );

        assert_eq!(result, Err(expected_reason.to_string()));
        assert_eq!(waits.get(), 0);
    }

    #[test]
    fn active_diagnostics_parse_without_inference() {
        let visible = parse_visible_labels(active_labels()).expect("active labels");
        assert_eq!(visible.collector_state, DesktopCollectorState::Active);
        assert_eq!(
            visible.privileged_source,
            DesktopPrivilegedSource::InstalledCollectorService
        );
        assert!(!visible.standard_monitoring_current);
        assert!(visible.protected_sample_current);
        assert!(!visible.fallback_etw_disabled);
    }

    #[test]
    fn header_diagnostics_matcher_accepts_stateful_names_only() {
        for name in [
            "Telemetry healthy. Open diagnostics.",
            "Telemetry stale. Open diagnostics.",
            "Telemetry paused. Open diagnostics.",
            "2 limitations. Open diagnostics.",
            "App resource warning. Open diagnostics.",
        ] {
            assert!(is_header_diagnostics_button_name(name), "{name}");
        }
        for name in [
            "",
            ". Open diagnostics.",
            "Open diagnostics",
            "Open diagnostics from resource rail",
            "Diagnostics Healthy",
            "Diagnostics Warning",
        ] {
            assert!(!is_header_diagnostics_button_name(name), "{name}");
        }
    }

    #[test]
    fn unverified_fallback_etw_is_retryable_until_the_deadline() {
        let labels = active_labels()
            .into_iter()
            .map(|label| {
                if label.starts_with("Fallback process ETW:") {
                    "Fallback process ETW: Not verified".to_string()
                } else {
                    label
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(
            classify_visible_labels(DesktopPhase::FinalPrimary, labels),
            VisibleRead::Retryable("lifecycle_desktop_fallback_etw_truth_unavailable".to_string())
        );
    }

    #[test]
    fn visible_poll_does_not_retry_active_identity_failures() {
        for (key, value) in [
            ("Service version", "9.9.9"),
            ("Service release", "9.9.9"),
            ("Service protocol", "2"),
            ("Minimum desktop", "9.9.9"),
            ("Service instance", "invalid-instance"),
            ("Service detail", "unexpected_active_detail"),
        ] {
            assert_visible_poll_stops_without_wait(
                DesktopPhase::FinalPrimary,
                replace_label(active_labels(), key, value),
                "lifecycle_desktop_active_identity_invalid",
            );
        }
    }

    #[test]
    fn visible_poll_does_not_retry_incompatible_identity_failures() {
        assert_visible_poll_stops_without_wait(
            DesktopPhase::FinalIncompatibleService,
            replace_label(
                incompatible_service_labels(),
                "Service detail",
                "collector_service_failed",
            ),
            "lifecycle_desktop_incompatible_identity_invalid",
        );
    }

    #[test]
    fn visible_poll_does_not_retry_fallback_identity_failures() {
        assert_visible_poll_stops_without_wait(
            DesktopPhase::FinalMissingService,
            replace_label(
                missing_service_labels(),
                "Service detail",
                "collector_service_stopped",
            ),
            "lifecycle_desktop_fallback_identity_invalid",
        );
    }

    #[test]
    fn visible_poll_does_not_retry_terminal_collector_states() {
        for (state, reason) in [
            (
                "Collector service unauthorized",
                "lifecycle_desktop_visible_collector_unauthorized",
            ),
            (
                "Collector service failed",
                "lifecycle_desktop_visible_collector_failed",
            ),
        ] {
            assert_visible_poll_stops_without_wait(
                DesktopPhase::FinalPrimary,
                replace_label(active_labels(), "Collector service", state),
                reason,
            );
        }
    }

    #[test]
    fn visible_poll_retries_missing_and_connecting_no_sample_until_active() {
        let mut reads = VecDeque::from([Vec::new(), connecting_labels(), active_labels()]);
        let waits = Cell::new(0);
        let visible = poll_visible_contract(
            DesktopPhase::FinalPrimary,
            || Ok(reads.pop_front().expect("scripted read")),
            || false,
            || waits.set(waits.get() + 1),
        )
        .expect("active state becomes ready");

        assert_eq!(visible.collector_state, DesktopCollectorState::Active);
        assert_eq!(waits.get(), 2);
    }

    #[test]
    fn visible_poll_retries_connecting_no_sample_until_fallback() {
        let mut reads = VecDeque::from([connecting_labels(), missing_service_labels()]);
        let waits = Cell::new(0);
        let visible = poll_visible_contract(
            DesktopPhase::FinalMissingService,
            || Ok(reads.pop_front().expect("scripted read")),
            || false,
            || waits.set(waits.get() + 1),
        )
        .expect("fallback state becomes ready");

        assert_eq!(visible.collector_state, DesktopCollectorState::NotInstalled);
        assert_eq!(waits.get(), 1);
    }

    #[test]
    fn visible_poll_times_out_on_a_permanently_wrong_state() {
        let deadline_checks = Cell::new(0);
        let waits = Cell::new(0);
        let result = poll_visible_contract(
            DesktopPhase::FinalMissingService,
            || Ok(active_labels()),
            || {
                let checks = deadline_checks.get() + 1;
                deadline_checks.set(checks);
                checks >= 2
            },
            || waits.set(waits.get() + 1),
        );

        assert_eq!(
            result,
            Err(
                "lifecycle_desktop_visible_state_timeout:lifecycle_desktop_visible_state_invalid"
                    .to_string()
            )
        );
        assert_eq!(waits.get(), 1);
    }

    #[test]
    fn visible_poll_does_not_retry_process_identity_failure() {
        let waits = Cell::new(0);
        let result = poll_visible_contract(
            DesktopPhase::FinalPrimary,
            || {
                Ok(active_labels()
                    .into_iter()
                    .map(|label| {
                        if label.starts_with("Current process:") {
                            "Current process: Elevated token".to_string()
                        } else {
                            label
                        }
                    })
                    .collect())
            },
            || false,
            || waits.set(waits.get() + 1),
        );

        assert_eq!(
            result,
            Err("lifecycle_desktop_visible_process_not_standard".to_string())
        );
        assert_eq!(waits.get(), 0);
    }
}
