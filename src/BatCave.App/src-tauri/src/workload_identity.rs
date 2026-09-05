//! Workload membership requires both executable identity and observed process ancestry.
//! Display names never establish membership, and every aggregate key identifies its exact scope.
use std::{
    collections::{HashMap, HashSet},
    fmt::Write,
};

use sha2::{Digest, Sha256};

use crate::contracts::ProcessSample;

#[derive(Debug, Clone)]
pub(crate) struct WorkloadMembership {
    pub key: String,
    pub representative_index: usize,
}

/// Parent links are valid only within this sample's unambiguous PID generations.
/// Equal birth timestamps cannot establish order when a collector rounds time.
/// Every chain reaching a cycle is rejected, including descendants of that cycle.
pub(crate) fn verified_parent_indices(processes: &[ProcessSample]) -> Vec<Option<usize>> {
    let mut by_pid = HashMap::<u32, Vec<usize>>::new();
    for (index, process) in processes.iter().enumerate() {
        if let Some(pid) = process_pid(process) {
            by_pid.entry(pid).or_default().push(index);
        }
    }
    let parents = processes
        .iter()
        .map(|process| {
            let pid = process_pid(process)?;
            if by_pid.get(&pid)?.len() != 1 {
                return None;
            }
            let parent_pid = process.parent_pid.as_ref()?.parse::<u32>().ok()?;
            let [parent] = by_pid.get(&parent_pid)?.as_slice() else {
                return None;
            };
            Some(*parent)
        })
        .collect::<Vec<_>>();
    let mut acyclic = vec![None; processes.len()];
    for index in 0..processes.len() {
        if acyclic[index].is_some() {
            continue;
        }
        let mut trail = HashSet::new();
        let mut cursor = Some(index);
        let valid = loop {
            let Some(current) = cursor else {
                break true;
            };
            if let Some(known) = acyclic[current] {
                break known;
            }
            if !trail.insert(current) {
                break false;
            }
            cursor = parents[current];
        };
        for member in trail {
            acyclic[member] = Some(valid);
        }
    }
    parents
        .into_iter()
        .enumerate()
        .map(|(index, parent)| {
            let parent = parent?;
            (acyclic[index] == Some(true)
                && processes[index].start_time_ms > 0
                && processes[parent].start_time_ms > 0
                && processes[parent].start_time_ms < processes[index].start_time_ms)
                .then_some(parent)
        })
        .collect()
}

pub(crate) fn workload_memberships(processes: &[ProcessSample]) -> Vec<WorkloadMembership> {
    let parents = verified_parent_indices(processes);
    let executable_ids = processes
        .iter()
        .map(|process| executable_identity(&process.exe))
        .collect::<Vec<_>>();
    let mut resolved_roots = vec![None; processes.len()];
    for index in 0..processes.len() {
        let mut trail = Vec::new();
        let mut current = index;
        let root = loop {
            if let Some(root) = resolved_roots[current] {
                break root;
            }
            trail.push(current);
            let Some(parent) = parents[current] else {
                break current;
            };
            if executable_ids[current].is_none()
                || executable_ids[current] != executable_ids[parent]
            {
                break current;
            }
            current = parent;
        };
        for member in trail {
            resolved_roots[member] = Some(root);
        }
    }
    let roots = resolved_roots
        .into_iter()
        .enumerate()
        .map(|(index, root)| root.unwrap_or(index))
        .collect::<Vec<_>>();
    let mut members = HashMap::<usize, Vec<usize>>::new();
    for (index, root) in roots.iter().enumerate() {
        members.entry(*root).or_default().push(index);
    }
    let mut pid_counts = HashMap::<&str, usize>::new();
    for process in processes {
        *pid_counts.entry(&process.pid).or_default() += 1;
    }
    let keys = members
        .iter()
        .map(|(root, members)| {
            let mut identities = members
                .iter()
                .map(|index| {
                    let process = &processes[*index];
                    format!("{}:{}", process.pid, process.start_time_ms)
                })
                .collect::<Vec<_>>();
            identities.sort();
            let mut hash = Sha256::new();
            for value in std::iter::once(executable_ids[*root].as_deref().unwrap_or(""))
                .chain(identities.iter().map(String::as_str))
            {
                hash.update((value.len() as u64).to_le_bytes());
                hash.update(value.as_bytes());
            }
            // Ambiguous/unknown rows never join even when their visible identity fields match.
            if pid_counts[processes[*root].pid.as_str()] != 1 {
                hash.update(root.to_le_bytes());
            }
            let mut key = String::with_capacity(70);
            key.push_str("scope:");
            for byte in hash.finalize() {
                write!(&mut key, "{byte:02x}").expect("string write");
            }
            (*root, key)
        })
        .collect::<HashMap<_, _>>();
    roots
        .into_iter()
        .map(|root| WorkloadMembership {
            key: keys[&root].clone(),
            representative_index: root,
        })
        .collect()
}

fn process_pid(process: &ProcessSample) -> Option<u32> {
    process.pid.parse::<u32>().ok().filter(|pid| *pid > 0)
}

/// Normalize syntax only: no name heuristics, filesystem traversal, or guessed executable family.
fn executable_identity(exe: &str) -> Option<String> {
    if exe.is_empty() || exe.chars().any(char::is_control) {
        return None;
    }
    let windows = exe.as_bytes().get(1) == Some(&b':') || exe.starts_with("\\\\");
    let normalized = if windows {
        exe.replace('\\', "/")
    } else {
        exe.to_string()
    };
    let absolute =
        normalized.starts_with('/') || (windows && normalized.as_bytes().get(2) == Some(&b'/'));
    if !absolute {
        return None;
    }
    let parts = normalized
        .split('/')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    if parts.iter().any(|part| matches!(*part, "." | "..")) {
        return None;
    }
    // Preserve case, including Windows paths: per-directory case sensitivity is not in the evidence.
    if !windows {
        if let Some(index) = parts
            .windows(2)
            .position(|parts| parts[0].ends_with(".app") && parts[1] == "Contents")
        {
            let relative = &parts[index + 2..];
            let main_executable = relative.len() == 2 && relative[0] == "MacOS";
            let nested_helper = relative.first() == Some(&"Frameworks")
                && relative.len() >= 5
                && relative[relative.len() - 2] == "MacOS"
                && relative[relative.len() - 3] == "Contents"
                && relative[relative.len() - 4].ends_with(".app");
            if main_executable || nested_helper {
                return Some(format!("bundle:/{}", parts[..=index].join("/")));
            }
        }
    }
    Some(format!("exe:{normalized}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn process(pid: &str, parent: Option<&str>, start: u64, exe: &str) -> ProcessSample {
        serde_json::from_value(serde_json::json!({
            "pid": pid, "parent_pid": parent, "start_time_ms": start, "name": "same name",
            "exe": exe, "status": "running", "cpu_percent": 0.0, "memory_bytes": 0,
            "private_bytes": 0, "io_read_total_bytes": 0, "io_write_total_bytes": 0,
            "io_read_bps": 0, "io_write_bps": 0, "threads": 1, "handles": 0, "access_state": "full"
        }))
        .unwrap()
    }
    #[test]
    fn independent_roots_and_different_executables_stay_separate() {
        for exe in ["/usr/bin/node", "/usr/bin/python"] {
            let rows = [
                process("1", None, 1, exe),
                process("2", None, 2, exe),
                process("3", Some("1"), 3, "/other/node"),
            ];
            let ids = workload_memberships(&rows);
            assert_ne!(ids[0].key, ids[1].key);
            assert_ne!(ids[0].key, ids[2].key);
        }
    }
    #[test]
    fn verified_same_executable_descendants_form_one_scope() {
        let rows = [
            process("1", None, 1, "/usr/bin/node"),
            process("2", Some("1"), 2, "/usr/bin/node"),
            process("3", Some("2"), 3, "/usr/bin/node"),
        ];
        let ids = workload_memberships(&rows);
        assert!(ids.iter().all(|id| id.key == ids[0].key));
        assert_eq!(ids[2].representative_index, 0);
        assert_ne!(ids[2].representative_index, 2);
        let reordered = [rows[2].clone(), rows[0].clone(), rows[1].clone()];
        assert_eq!(workload_memberships(&reordered)[0].key, ids[0].key);
        assert_ne!(workload_memberships(&rows[..2])[0].key, ids[0].key);
    }
    #[test]
    fn same_timestamp_cannot_verify_parent_generation() {
        for start in [1, 1_000, 1_700_000_000_000] {
            let rows = [
                process("1", None, start, "/usr/bin/node"),
                process("2", Some("1"), start, "/usr/bin/node"),
            ];
            assert_eq!(verified_parent_indices(&rows), vec![None, None]);
            let ids = workload_memberships(&rows);
            assert_ne!(ids[0].key, ids[1].key);
            assert_eq!(ids[1].representative_index, 1);
        }
    }
    #[test]
    fn bundle_helpers_require_the_same_outer_bundle_and_ancestry() {
        let rows = [
            process("1", None, 1, "/Applications/Code.app/Contents/MacOS/Code"),
            process("2", Some("1"), 2, "/Applications/Code.app/Contents/Frameworks/Code Helper.app/Contents/MacOS/Code Helper"),
            process("3", Some("1"), 3, "/Other/Code.app/Contents/MacOS/Code"),
        ];
        let ids = workload_memberships(&rows);
        assert_eq!(ids[0].key, ids[1].key);
        assert_ne!(ids[0].key, ids[2].key);
    }
    #[test]
    fn unknown_or_contradictory_identity_never_joins() {
        for rows in [
            vec![
                process("1", None, 0, "/node"),
                process("2", Some("1"), 2, "/node"),
            ],
            vec![process("1", None, 1, ""), process("2", Some("1"), 2, "")],
            vec![
                process("1", None, 3, "/node"),
                process("2", Some("1"), 2, "/node"),
            ],
            vec![
                process("1", None, 1, "/node"),
                process("2", Some("9"), 2, "/node"),
            ],
            vec![
                process("1", None, 1, "/node"),
                process("1", None, 2, "/node"),
                process("2", Some("1"), 3, "/node"),
            ],
            vec![
                process("1", Some("2"), 1, "/node"),
                process("2", Some("1"), 1, "/node"),
                process("3", Some("1"), 2, "/node"),
            ],
            vec![
                process("1", Some("1"), 1, "/node"),
                process("2", Some("1"), 2, "/node"),
            ],
        ] {
            let keys = workload_memberships(&rows)
                .into_iter()
                .map(|id| id.key)
                .collect::<HashSet<_>>();
            assert_eq!(keys.len(), rows.len(), "{rows:?}");
        }
    }
    #[test]
    fn path_normalization_does_not_guess_names_or_resolve_dot_segments() {
        assert_eq!(
            executable_identity("C:\\App\\node.exe"),
            executable_identity("C:/App/node.exe")
        );
        assert_ne!(
            executable_identity("/app/node"),
            executable_identity("/App/node")
        );
        assert_eq!(executable_identity("/app/../node"), None);
        assert_eq!(executable_identity("node"), None);
    }
}
