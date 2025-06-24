use crate::daemon::process::proxy::ProxyInfo;
use dashmap::DashMap;
use std::sync::Arc;

/// Registry for managing process lookups
#[derive(Clone)]
pub struct ProcessRegistry {
    processes: Arc<DashMap<String, Arc<ProxyInfo>>>,
}

impl ProcessRegistry {
    pub fn new() -> Self {
        Self {
            processes: Arc::new(DashMap::new()),
        }
    }

    /// Add a process to the registry
    pub fn add_process(&self, proxy: Arc<ProxyInfo>) {
        self.processes.insert(proxy.id.clone(), proxy);
    }

    /// Remove a process from the registry
    pub fn remove_process(&self, id: &str) -> Option<Arc<ProxyInfo>> {
        self.processes.remove(id).map(|(_, v)| v)
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
        self.get_process_by_id(name_or_id)
            .or_else(|| self.get_process_by_name(name_or_id))
    }

    /// Get process by name or ID with project filter
    pub fn get_process_by_name_or_id_with_project(
        &self,
        name_or_id: &str,
        project: Option<&str>,
    ) -> Option<Arc<ProxyInfo>> {
        if let Some(project_name) = project {
            // First try ID lookup
            if let Some(process) = self.get_process_by_id(name_or_id) {
                if process.project == project_name {
                    return Some(process);
                }
            }
            // Then try name lookup with project filter
            self.processes
                .iter()
                .find(|entry| entry.name == name_or_id && entry.project == project_name)
                .map(|entry| entry.value().clone())
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
