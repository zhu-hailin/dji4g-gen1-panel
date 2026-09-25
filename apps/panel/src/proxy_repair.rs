//! The only production port allowed to touch a known proxy client's configuration.

use std::{
    fs,
    sync::Mutex,
    time::{Duration, SystemTime},
};

use dji4g_application::{
    HostNetworkPort, PortError, PortFuture, ProxyRepairPreview, ProxyRepairResult,
};
use dji4g_domain::{
    ErrorCode, HostNetworkFinding, HostNetworkObservation, classify_host_network,
    host_observation_is_fresh, sha256,
};
use dji4g_windows_platform::{
    host_network::{inspect_host_network, roaming_app_data},
    proxy_clients::clash_verge_rev::{self as clash, BackupRecord, RepairCandidate},
};

const CLASH_HOME: &str = "io.github.clash-verge-rev.clash-verge-rev";
// Keep the production writer closed until the official v2.5.5 isolated-client
// repair/restart/recheck/restore acceptance has been recorded.
const AUTOMATIC_REPAIR_ACCEPTED: bool = false;

#[derive(Default)]
struct State {
    last_observation: Option<HostNetworkObservation>,
    candidate: Option<RepairCandidate>,
    prepared: Option<Prepared>,
    backup: Option<(u64, BackupRecord)>,
    next_id: u64,
}

struct Prepared {
    id: u64,
    expires_at: SystemTime,
    topology: [u8; 32],
    candidate: RepairCandidate,
}

#[derive(Default)]
pub struct ProductionHostNetwork {
    state: Mutex<State>,
}

fn error(code: &'static str) -> PortError {
    PortError::new(ErrorCode::CapabilityUnavailable, code)
}

fn topology_digest(observation: &HostNetworkObservation) -> [u8; 32] {
    let mut rows = observation
        .adapters
        .iter()
        .map(|adapter| {
            format!(
                "{}:{}:{}:{}",
                adapter.guid, adapter.luid, adapter.alias, adapter.up
            )
        })
        .collect::<Vec<_>>();
    rows.extend(
        observation
            .default_routes
            .iter()
            .map(|route| format!("route:{:?}:{}:{}", route.family, route.luid, route.metric)),
    );
    rows.sort();
    let mut material = Vec::new();
    for row in rows {
        material.extend_from_slice(&(row.len() as u64).to_be_bytes());
        material.extend_from_slice(row.as_bytes());
    }
    sha256(&material)
}

fn scan() -> Result<(HostNetworkObservation, Option<RepairCandidate>), PortError> {
    let mut observation = inspect_host_network().map_err(|e| error(e.code))?;
    let root = roaming_app_data()
        .map_err(|e| error(e.code))?
        .join(CLASH_HOME);
    match fs::metadata(&root) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            observation.proxy_inspection_complete = true;
            return Ok((observation, None));
        }
        Err(_) => {
            observation.proxy_error_code = Some("proxy:config_unreadable".into());
            return Ok((observation, None));
        }
        Ok(_) => {}
    }
    let processes = match clash::running_processes() {
        Ok(processes) => processes,
        Err(e) => {
            observation.proxy_error_code = Some(e.0.into());
            return Ok((observation, None));
        }
    };
    let client_paths = processes
        .iter()
        .filter(|(name, _)| name == "clash-verge.exe")
        .filter_map(|(_, path)| path.as_ref())
        .collect::<Vec<_>>();
    let executable = match client_paths.as_slice() {
        [path] => Some((*path).clone()),
        _ => None,
    };
    let version = executable
        .as_deref()
        .and_then(|path| clash::executable_version(path).ok());
    match clash::inspect_directory(&root, version.as_deref()) {
        Ok(mut inspected) => {
            if !AUTOMATIC_REPAIR_ACCEPTED {
                inspected.candidate = None;
                if let Some(binding) = &mut inspected.binding {
                    binding.repairable = false;
                }
            }
            if let Some(candidate) = &mut inspected.candidate {
                candidate.executable = executable;
            }
            observation.binding = inspected.binding;
            observation.proxy_inspection_complete = true;
            Ok((observation, inspected.candidate))
        }
        Err(e) => {
            observation.proxy_error_code = Some(e.0.into());
            Ok((observation, None))
        }
    }
}

impl ProductionHostNetwork {
    fn inspect_now(&self) -> Result<HostNetworkObservation, PortError> {
        let (observation, candidate) = scan()?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| error("proxy:state_unavailable"))?;
        state.last_observation = Some(observation.clone());
        state.candidate = candidate;
        state.prepared = None;
        Ok(observation)
    }

    fn prepare_now(&self, _finding_id: u64) -> Result<ProxyRepairPreview, PortError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| error("proxy:state_unavailable"))?;
        let last = state
            .last_observation
            .as_ref()
            .ok_or_else(|| error("proxy:check_required"))?;
        if !host_observation_is_fresh(last.observed_at, SystemTime::now())
            || classify_host_network(last) != HostNetworkFinding::MissingBoundInterface
        {
            return Err(error("proxy:check_required"));
        }
        let candidate = state
            .candidate
            .as_ref()
            .ok_or_else(|| error("proxy:repair_unsupported"))?
            .clone();
        let previous_topology = topology_digest(last);
        let (fresh, fresh_candidate) = scan()?;
        if topology_digest(&fresh) != previous_topology
            || classify_host_network(&fresh) != HostNetworkFinding::MissingBoundInterface
            || fresh_candidate.as_ref().is_none_or(|next| {
                next.path != candidate.path
                    || next.alias != candidate.alias
                    || next.before != candidate.before
            })
        {
            return Err(error("proxy:evidence_changed"));
        }
        clash::verify_candidate(&candidate).map_err(|e| error(e.0))?;
        state.next_id = state.next_id.saturating_add(1);
        let id = state.next_id;
        let expires_at = SystemTime::now() + Duration::from_secs(60);
        state.prepared = Some(Prepared {
            id,
            expires_at,
            topology: previous_topology,
            candidate: candidate.clone(),
        });
        Ok(ProxyRepairPreview {
            plan_id: id,
            interface_alias: candidate.alias,
            expires_at,
        })
    }

    fn apply_now(&self, id: u64) -> Result<ProxyRepairResult, PortError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| error("proxy:state_unavailable"))?;
        let plan = state
            .prepared
            .take()
            .ok_or_else(|| error("proxy:plan_missing"))?;
        if plan.id != id || SystemTime::now() > plan.expires_at {
            return Err(error("proxy:plan_expired"));
        }
        let fresh = inspect_host_network().map_err(|e| error(e.code))?;
        if topology_digest(&fresh) != plan.topology {
            return Err(error("proxy:evidence_changed"));
        }
        let processes = clash::running_processes().map_err(|e| error(e.0))?;
        if !processes.is_empty() {
            return Err(error("proxy:exit_client_first"));
        }
        let executable = plan
            .candidate
            .executable
            .as_deref()
            .ok_or_else(|| error("proxy:client_version_unknown"))?;
        if clash::executable_version(executable).map_err(|e| error(e.0))? != "2.5.5" {
            return Err(error("proxy:client_changed"));
        }
        clash::verify_candidate(&plan.candidate).map_err(|e| error(e.0))?;
        let backup = clash::apply_candidate(&plan.candidate).map_err(|e| error(e.0))?;
        state.next_id = state.next_id.saturating_add(1);
        let backup_id = state.next_id;
        state.backup = Some((backup_id, backup));
        Ok(ProxyRepairResult {
            backup_id,
            changed: true,
        })
    }

    fn restore_now(&self, id: u64) -> Result<(), PortError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| error("proxy:state_unavailable"))?;
        let (_, record) = state
            .backup
            .as_ref()
            .filter(|(backup_id, _)| *backup_id == id)
            .ok_or_else(|| error("proxy:backup_unavailable"))?;
        if !clash::running_processes()
            .map_err(|e| error(e.0))?
            .is_empty()
        {
            return Err(error("proxy:exit_client_first"));
        }
        clash::restore_candidate(record).map_err(|e| error(e.0))?;
        state.backup = None;
        Ok(())
    }
}

impl HostNetworkPort for ProductionHostNetwork {
    fn inspect(&self) -> PortFuture<'_, Result<HostNetworkObservation, PortError>> {
        Box::pin(async { self.inspect_now() })
    }
    fn prepare_repair(
        &self,
        finding_id: u64,
    ) -> PortFuture<'_, Result<ProxyRepairPreview, PortError>> {
        Box::pin(async move { self.prepare_now(finding_id) })
    }
    fn apply_repair(&self, plan_id: u64) -> PortFuture<'_, Result<ProxyRepairResult, PortError>> {
        Box::pin(async move { self.apply_now(plan_id) })
    }
    fn restore_repair(&self, backup_id: u64) -> PortFuture<'_, Result<(), PortError>> {
        Box::pin(async move { self.restore_now(backup_id) })
    }
}
