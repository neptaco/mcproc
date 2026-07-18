use crate::common::process_key::ProcessKey;
use crate::daemon::process::proxy::ProxyInfo;
use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use std::sync::Arc;

/// Registry for managing process lookups
#[derive(Clone)]
pub struct ProcessRegistry {
    processes: Arc<DashMap<String, Arc<ProxyInfo>>>,
    reserved_names: Arc<DashMap<ProcessKey, ()>>,
}

impl ProcessRegistry {
    pub fn new() -> Self {
        Self {
            processes: Arc::new(DashMap::new()),
            reserved_names: Arc::new(DashMap::new()),
        }
    }

    /// Add a process to the registry
    pub fn add_process(&self, proxy: Arc<ProxyInfo>) {
        self.processes.insert(proxy.id.clone(), proxy);
    }

    /// Remove a process from the registry
    pub fn remove_process(&self, id: &str) -> Option<Arc<ProxyInfo>> {
        self.processes.remove(id).map(|(_, proxy)| {
            self.release_name(&proxy.key);
            proxy
        })
    }

    pub fn try_reserve_name(&self, key: ProcessKey) -> bool {
        match self.reserved_names.entry(key) {
            Entry::Vacant(entry) => {
                entry.insert(());
                true
            }
            Entry::Occupied(_) => false,
        }
    }

    pub fn release_name(&self, key: &ProcessKey) {
        self.reserved_names.remove(key);
    }

    pub fn get_process_by_name_with_project(
        &self,
        name: &str,
        project: &str,
    ) -> Option<Arc<ProxyInfo>> {
        self.processes
            .iter()
            .find(|entry| entry.name == name && entry.project == project)
            .map(|entry| entry.value().clone())
    }

    /// Get process by ID
    pub fn get_process_by_id(&self, id: &str) -> Option<Arc<ProxyInfo>> {
        self.processes.get(id).map(|entry| entry.clone())
    }

    /// Get process by name
    pub fn get_process_by_name(&self, name: &str) -> Option<Arc<ProxyInfo>> {
        self.processes
            .iter()
            .find(|entry| entry.name == name)
            .map(|entry| entry.value().clone())
    }

    /// Get process by name or ID
    pub fn get_process_by_name_or_id(&self, name_or_id: &str) -> Option<Arc<ProxyInfo>> {
        self.get_process_by_name(name_or_id)
            .or_else(|| self.get_process_by_id(name_or_id))
    }

    /// Get process by name or ID with project filter
    pub fn get_process_by_name_or_id_with_project(
        &self,
        name_or_id: &str,
        project: Option<&str>,
    ) -> Option<Arc<ProxyInfo>> {
        if let Some(project_name) = project {
            if let Some(process) = self.get_process_by_name_with_project(name_or_id, project_name) {
                return Some(process);
            }
            if let Some(process) = self.get_process_by_id(name_or_id) {
                if process.project == project_name {
                    return Some(process);
                }
            }
            None
        } else {
            self.get_process_by_name_or_id(name_or_id)
        }
    }

    /// Get all processes
    pub fn get_all_processes(&self) -> Vec<Arc<ProxyInfo>> {
        self.processes
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// List processes (same as get_all_processes for compatibility)
    pub fn list_processes(&self) -> Vec<Arc<ProxyInfo>> {
        self.get_all_processes()
    }

    /// Get processes by project
    pub fn get_processes_by_project(&self, project: &str) -> Vec<Arc<ProxyInfo>> {
        self.processes
            .iter()
            .filter(|entry| entry.project == project)
            .map(|entry| entry.value().clone())
            .collect()
    }

    /// Get all project names
    pub fn get_all_projects(&self) -> Vec<String> {
        let mut projects: Vec<String> = self
            .processes
            .iter()
            .map(|entry| entry.project.clone())
            .collect();
        projects.sort();
        projects.dedup();
        projects
    }
}

impl Default for ProcessRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::ProcessRegistry;
    use crate::daemon::process::proxy::ProxyInfo;
    use crate::daemon::process::types::ProxyInfoParams;
    use std::sync::Arc;

    fn proxy(id: &str, name: &str, project: &str) -> Arc<ProxyInfo> {
        Arc::new(ProxyInfo::new(ProxyInfoParams {
            id: id.to_string(),
            name: name.to_string(),
            project: project.to_string(),
            cmd: None,
            args: Vec::new(),
            cwd: None,
            env: None,
            wait_for_log: None,
            wait_timeout: None,
            toolchain: None,
            pid: 0,
        }))
    }

    #[test]
    fn name_lookup_wins_over_uuid_lookup() {
        let registry = ProcessRegistry::new();
        let process_a = proxy("ambiguous", "process-a", "project");
        let process_b = proxy("process-b-id", "ambiguous", "project");
        registry.add_process(process_a);
        registry.add_process(process_b.clone());

        assert_eq!(
            registry
                .get_process_by_name_or_id_with_project("ambiguous", Some("project"))
                .unwrap()
                .id,
            process_b.id
        );
        assert_eq!(
            registry
                .get_process_by_name_or_id_with_project("ambiguous", None)
                .unwrap()
                .id,
            process_b.id
        );
    }
}
