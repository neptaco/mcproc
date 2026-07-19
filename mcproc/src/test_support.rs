use crate::common::config::Config;
use crate::daemon::api::grpc::service::GrpcService;
use crate::daemon::log::LogHub;
use crate::daemon::process::ProcessManager;
use crate::daemon::stream::StreamEventHub;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

pub(crate) struct ProcessTestFixture {
    config: Arc<Config>,
    event_hub: Arc<StreamEventHub>,
    log_hub: Arc<LogHub>,
    pub process_manager: Arc<ProcessManager>,
    pub root: PathBuf,
}

impl ProcessTestFixture {
    pub fn new(prefix: &str, process_stop_timeout_ms: u64) -> Self {
        let root = PathBuf::from("/tmp").join(format!("{prefix}-{}", uuid::Uuid::new_v4()));
        let mut config = Config::default();
        config.paths.data_dir = root.join("data");
        config.paths.log_dir = root.join("log");
        config.paths.socket_path = root.join("runtime/mcprocd.sock");
        config.paths.pid_file = root.join("runtime/mcprocd.pid");
        config.paths.daemon_log_file = root.join("state/mcprocd.log");
        config.process.restart.delay_ms = 0;
        config.process.restart.process_stop_timeout_ms = process_stop_timeout_ms;

        create_test_directories(&config);

        let config = Arc::new(config);
        let event_hub = Arc::new(StreamEventHub::new());
        let log_hub = Arc::new(LogHub::with_event_hub(config.clone(), event_hub.clone()));
        let process_manager = Arc::new(ProcessManager::with_event_hub(
            config.clone(),
            log_hub.clone(),
            event_hub.clone(),
        ));

        Self {
            config,
            event_hub,
            log_hub,
            process_manager,
            root,
        }
    }

    pub fn grpc_service(&self) -> GrpcService {
        GrpcService::new(
            self.process_manager.clone(),
            self.log_hub.clone(),
            self.config.clone(),
            self.event_hub.clone(),
        )
    }

    pub fn socket_path(&self) -> PathBuf {
        self.config.paths.socket_path.clone()
    }

    pub async fn stop_all(&self) {
        for process in self.process_manager.get_all_processes() {
            let stop = self
                .process_manager
                .stop_process(&process.id, Some(&process.project), true);
            tokio::time::timeout(Duration::from_secs(15), stop)
                .await
                .expect("managed process stop exceeded cleanup deadline")
                .expect("managed process cleanup failed");
        }
    }

    pub fn remove_root(&mut self) {
        std::fs::remove_dir_all(&self.root).unwrap();
        self.root = PathBuf::new();
    }
}

impl Drop for ProcessTestFixture {
    fn drop(&mut self) {
        if self.root.as_os_str().is_empty() {
            return;
        }

        kill_process_groups_and_reap(&self.process_manager);
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn create_test_directories(config: &Config) {
    std::fs::create_dir_all(&config.paths.data_dir).unwrap();
    std::fs::create_dir_all(&config.paths.log_dir).unwrap();
    std::fs::create_dir_all(config.paths.socket_path.parent().unwrap()).unwrap();
    std::fs::create_dir_all(config.paths.daemon_log_file.parent().unwrap()).unwrap();
}

fn kill_process_groups_and_reap(process_manager: &ProcessManager) {
    #[cfg(unix)]
    {
        let processes = process_manager.get_all_processes();
        for process in &processes {
            unsafe {
                libc::kill(-(process.pid as i32), libc::SIGKILL);
            }
        }

        for process in processes {
            reap_process(process.pid);
        }
    }
}

#[cfg(unix)]
fn reap_process(pid: u32) {
    for _ in 0..50 {
        let result = unsafe { libc::waitpid(pid as i32, std::ptr::null_mut(), libc::WNOHANG) };
        if result != 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}
